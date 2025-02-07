// Copyright 2019, The Tari Project
//
// Redistribution and use in source and binary forms, with or without modification, are permitted provided that the
// following conditions are met:
//
// 1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following
// disclaimer.
//
// 2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the
// following disclaimer in the documentation and/or other materials provided with the distribution.
//
// 3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote
// products derived from this software without specific prior written permission.
//
// THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES,
// INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
// DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
// SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
// SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY,
// WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE
// USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

#[cfg(feature = "rpc")]
use crate::protocol::rpc::{
    NamedProtocolService,
    RpcClient,
    RpcClientBuilder,
    RpcClientPool,
    RpcError,
    RpcPoolClient,
    RPC_MAX_FRAME_SIZE,
};

use super::{
    error::{ConnectionManagerError, PeerConnectionError},
    manager::ConnectionManagerEvent,
    types::ConnectionDirection,
};
use crate::{
    framing,
    framing::CanonicalFraming,
    multiplexing::{Control, IncomingSubstreams, Substream, Yamux},
    peer_manager::{NodeId, PeerFeatures},
    protocol::{ProtocolId, ProtocolNegotiation},
    runtime,
    utils::atomic_ref_counter::AtomicRefCounter,
};
use log::*;
use multiaddr::Multiaddr;
use std::{
    fmt,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};
use tokio::{
    sync::{mpsc, oneshot},
    time,
};
use tokio_stream::StreamExt;
use tracing::{self, span, Instrument, Level, Span};

const LOG_TARGET: &str = "comms::connection_manager::peer_connection";

static ID_COUNTER: AtomicUsize = AtomicUsize::new(0);

#[allow(clippy::too_many_arguments)]
pub fn create(
    connection: Yamux,
    peer_addr: Multiaddr,
    peer_node_id: NodeId,
    peer_features: PeerFeatures,
    direction: ConnectionDirection,
    event_notifier: mpsc::Sender<ConnectionManagerEvent>,
    our_supported_protocols: Vec<ProtocolId>,
    their_supported_protocols: Vec<ProtocolId>,
) -> Result<PeerConnection, ConnectionManagerError> {
    trace!(
        target: LOG_TARGET,
        "(Peer={}) Socket successfully upgraded to multiplexed socket",
        peer_node_id.short_str()
    );
    // All requests are request/response, so a channel size of 1 is all that is needed
    let (peer_tx, peer_rx) = mpsc::channel(1);
    let id = ID_COUNTER.fetch_add(1, Ordering::Relaxed); // Monotonic
    let substream_counter = connection.substream_counter();
    let peer_conn = PeerConnection::new(
        id,
        peer_tx,
        peer_node_id.clone(),
        peer_features,
        peer_addr,
        direction,
        substream_counter,
    );
    let peer_actor = PeerConnectionActor::new(
        id,
        peer_node_id,
        direction,
        connection,
        peer_rx,
        event_notifier,
        our_supported_protocols,
        their_supported_protocols,
    );
    runtime::current().spawn(peer_actor.run());

    Ok(peer_conn)
}

#[derive(Debug)]
pub enum PeerConnectionRequest {
    /// Open a new substream and negotiate the given protocol
    OpenSubstream {
        protocol_id: ProtocolId,
        reply_tx: oneshot::Sender<Result<NegotiatedSubstream<Substream>, PeerConnectionError>>,
        tracing_id: Option<tracing::span::Id>,
    },
    /// Disconnect all substreams and close the transport connection
    Disconnect(bool, oneshot::Sender<Result<(), PeerConnectionError>>),
}

pub type ConnectionId = usize;

/// Request handle for an active peer connection
#[derive(Debug, Clone)]
pub struct PeerConnection {
    id: ConnectionId,
    peer_node_id: NodeId,
    peer_features: PeerFeatures,
    request_tx: mpsc::Sender<PeerConnectionRequest>,
    address: Arc<Multiaddr>,
    direction: ConnectionDirection,
    started_at: Instant,
    substream_counter: AtomicRefCounter,
    handle_counter: Arc<()>,
}

impl PeerConnection {
    pub(crate) fn new(
        id: ConnectionId,
        request_tx: mpsc::Sender<PeerConnectionRequest>,
        peer_node_id: NodeId,
        peer_features: PeerFeatures,
        address: Multiaddr,
        direction: ConnectionDirection,
        substream_counter: AtomicRefCounter,
    ) -> Self {
        Self {
            id,
            request_tx,
            peer_node_id,
            peer_features,
            address: Arc::new(address),
            direction,
            started_at: Instant::now(),
            substream_counter,
            handle_counter: Arc::new(()),
        }
    }

    pub fn peer_node_id(&self) -> &NodeId {
        &self.peer_node_id
    }

    pub fn peer_features(&self) -> PeerFeatures {
        self.peer_features
    }

    pub fn direction(&self) -> ConnectionDirection {
        self.direction
    }

    pub fn address(&self) -> &Multiaddr {
        &self.address
    }

    pub fn id(&self) -> ConnectionId {
        self.id
    }

    pub fn is_connected(&self) -> bool {
        !self.request_tx.is_closed()
    }

    pub fn age(&self) -> Duration {
        self.started_at.elapsed()
    }

    pub fn substream_count(&self) -> usize {
        self.substream_counter.get()
    }

    pub fn handle_count(&self) -> usize {
        Arc::strong_count(&self.handle_counter)
    }

    #[tracing::instrument("peer_connection::open_substream", skip(self))]
    pub async fn open_substream(
        &mut self,
        protocol_id: &ProtocolId,
    ) -> Result<NegotiatedSubstream<Substream>, PeerConnectionError> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.request_tx
            .send(PeerConnectionRequest::OpenSubstream {
                protocol_id: protocol_id.clone(),
                reply_tx,
                tracing_id: Span::current().id(),
            })
            .await?;
        reply_rx
            .await
            .map_err(|_| PeerConnectionError::InternalReplyCancelled)?
    }

    #[tracing::instrument("peer_connection::open_framed_substream", skip(self))]
    pub async fn open_framed_substream(
        &mut self,
        protocol_id: &ProtocolId,
        max_frame_size: usize,
    ) -> Result<CanonicalFraming<Substream>, PeerConnectionError> {
        let substream = self.open_substream(protocol_id).await?;
        Ok(framing::canonical(substream.stream, max_frame_size))
    }

    #[cfg(feature = "rpc")]
    #[tracing::instrument("peer_connection::connect_rpc", skip(self), fields(peer_node_id = self.peer_node_id.to_string().as_str()))]
    pub async fn connect_rpc<T>(&mut self) -> Result<T, RpcError>
    where T: From<RpcClient> + NamedProtocolService {
        self.connect_rpc_using_builder(Default::default()).await
    }

    #[cfg(feature = "rpc")]
    #[tracing::instrument("peer_connection::connect_rpc_with_builder", skip(self, builder))]
    pub async fn connect_rpc_using_builder<T>(&mut self, builder: RpcClientBuilder<T>) -> Result<T, RpcError>
    where T: From<RpcClient> + NamedProtocolService {
        let protocol = ProtocolId::from_static(T::PROTOCOL_NAME);
        debug!(
            target: LOG_TARGET,
            "Attempting to establish RPC protocol `{}` to peer `{}`",
            String::from_utf8_lossy(&protocol),
            self.peer_node_id
        );
        let framed = self.open_framed_substream(&protocol, RPC_MAX_FRAME_SIZE).await?;
        builder.with_protocol_id(protocol).connect(framed).await
    }

    /// Creates a new RpcClientPool that can be shared between tasks. The client pool will lazily establish up to
    /// `max_sessions` sessions and provides client session that is least used.
    #[cfg(feature = "rpc")]
    pub fn create_rpc_client_pool<T>(
        &self,
        max_sessions: usize,
        client_config: RpcClientBuilder<T>,
    ) -> RpcClientPool<T>
    where
        T: RpcPoolClient + From<RpcClient> + NamedProtocolService + Clone,
    {
        RpcClientPool::new(self.clone(), max_sessions, client_config)
    }

    /// Immediately disconnects the peer connection. This can only fail if the peer connection worker
    /// is shut down (and the peer is already disconnected)
    pub async fn disconnect(&mut self) -> Result<(), PeerConnectionError> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.request_tx
            .send(PeerConnectionRequest::Disconnect(false, reply_tx))
            .await?;
        reply_rx
            .await
            .map_err(|_| PeerConnectionError::InternalReplyCancelled)?
    }

    pub(crate) async fn disconnect_silent(&mut self) -> Result<(), PeerConnectionError> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.request_tx
            .send(PeerConnectionRequest::Disconnect(true, reply_tx))
            .await?;
        reply_rx
            .await
            .map_err(|_| PeerConnectionError::InternalReplyCancelled)?
    }
}

impl fmt::Display for PeerConnection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        write!(
            f,
            "Id: {}, Node ID: {}, Direction: {}, Peer Address: {}, Age: {:.0?}, #Substreams: {}, #Refs: {}",
            self.id,
            self.peer_node_id.short_str(),
            self.direction,
            self.address,
            self.age(),
            self.substream_count(),
            self.handle_count()
        )
    }
}

impl PartialEq for PeerConnection {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

/// Actor for an active connection to a peer.
struct PeerConnectionActor {
    id: ConnectionId,
    peer_node_id: NodeId,
    request_rx: mpsc::Receiver<PeerConnectionRequest>,
    direction: ConnectionDirection,
    incoming_substreams: IncomingSubstreams,
    control: Control,
    event_notifier: mpsc::Sender<ConnectionManagerEvent>,
    our_supported_protocols: Vec<ProtocolId>,
    their_supported_protocols: Vec<ProtocolId>,
}

impl PeerConnectionActor {
    #[allow(clippy::too_many_arguments)]
    fn new(
        id: ConnectionId,
        peer_node_id: NodeId,
        direction: ConnectionDirection,
        connection: Yamux,
        request_rx: mpsc::Receiver<PeerConnectionRequest>,
        event_notifier: mpsc::Sender<ConnectionManagerEvent>,
        our_supported_protocols: Vec<ProtocolId>,
        their_supported_protocols: Vec<ProtocolId>,
    ) -> Self {
        Self {
            id,
            peer_node_id,
            direction,
            control: connection.get_yamux_control(),
            incoming_substreams: connection.into_incoming(),
            request_rx,
            event_notifier,
            our_supported_protocols,
            their_supported_protocols,
        }
    }

    pub async fn run(mut self) {
        loop {
            tokio::select! {
                maybe_request = self.request_rx.recv() => {
                    match maybe_request {
                        Some(request) => self.handle_request(request).await,
                        None => {
                            debug!(target: LOG_TARGET, "[{}] All peer connection handled dropped closing the connection", self);
                            break;
                        }
                    }
                },

                maybe_substream = self.incoming_substreams.next() => {
                    match maybe_substream {
                        Some(substream) => {
                            if let Err(err) = self.handle_incoming_substream(substream).await {
                                error!(
                                    target: LOG_TARGET,
                                    "[{}] Incoming substream for peer '{}' failed to open because '{error}'",
                                    self,
                                    self.peer_node_id.short_str(),
                                    error = err
                                )
                            }
                        },
                        None => {
                            debug!(target: LOG_TARGET, "[{}] Peer '{}' closed the connection", self, self.peer_node_id.short_str());
                            break;
                        },
                    }
                }
            }
        }

        if let Err(err) = self.disconnect(false).await {
            warn!(
                target: LOG_TARGET,
                "[{}] Failed to politely close connection to peer '{}' because '{}'",
                self,
                self.peer_node_id.short_str(),
                err
            );
        }
        self.request_rx.close();
    }

    async fn handle_request(&mut self, request: PeerConnectionRequest) {
        use PeerConnectionRequest::*;
        match request {
            OpenSubstream {
                protocol_id,
                reply_tx,
                tracing_id,
            } => {
                let span = span!(Level::TRACE, "handle_request");
                span.follows_from(tracing_id);
                let result = self.open_negotiated_protocol_stream(protocol_id).instrument(span).await;
                log_if_error_fmt!(
                    target: LOG_TARGET,
                    reply_tx.send(result),
                    "Reply oneshot closed when sending reply",
                );
            },
            Disconnect(silent, reply_tx) => {
                debug!(
                    target: LOG_TARGET,
                    "[{}] Disconnect{}requested for {} connection to peer '{}'",
                    self,
                    if silent { " (silent) " } else { " " },
                    self.direction,
                    self.peer_node_id.short_str()
                );
                let _ = reply_tx.send(self.disconnect(silent).await);
            },
        }
    }

    #[tracing::instrument(skip(self, stream),fields(comms.direction="inbound"))]
    async fn handle_incoming_substream(&mut self, mut stream: Substream) -> Result<(), PeerConnectionError> {
        let selected_protocol = ProtocolNegotiation::new(&mut stream)
            .negotiate_protocol_inbound(&self.our_supported_protocols)
            .await?;

        self.notify_event(ConnectionManagerEvent::NewInboundSubstream(
            self.peer_node_id.clone(),
            selected_protocol,
            stream,
        ))
        .await;

        Ok(())
    }

    #[tracing::instrument(skip(self))]
    async fn open_negotiated_protocol_stream(
        &mut self,
        protocol: ProtocolId,
    ) -> Result<NegotiatedSubstream<Substream>, PeerConnectionError> {
        const PROTOCOL_NEGOTIATION_TIMEOUT: Duration = Duration::from_secs(10);
        debug!(
            target: LOG_TARGET,
            "[{}] Negotiating protocol '{}' on new substream for peer '{}'",
            self,
            String::from_utf8_lossy(&protocol),
            self.peer_node_id.short_str()
        );
        let mut stream = self.control.open_stream().await?;

        let mut negotiation = ProtocolNegotiation::new(&mut stream);

        let selected_protocol = if self.their_supported_protocols.contains(&protocol) {
            let fut = negotiation.negotiate_protocol_outbound_optimistic(&protocol);
            time::timeout(PROTOCOL_NEGOTIATION_TIMEOUT, fut).await??
        } else {
            let selected_protocols = [protocol];
            let fut = negotiation.negotiate_protocol_outbound(&selected_protocols);
            time::timeout(PROTOCOL_NEGOTIATION_TIMEOUT, fut).await??
        };

        Ok(NegotiatedSubstream::new(selected_protocol, stream))
    }

    async fn notify_event(&mut self, event: ConnectionManagerEvent) {
        log_if_error!(
            target: LOG_TARGET,
            self.event_notifier.send(event).await,
            "Failed to send connection manager notification because '{}'",
        );
    }

    /// Disconnect this peer connection.
    ///
    /// # Arguments
    ///
    /// silent - true to suppress the PeerDisconnected event, false to publish the event
    async fn disconnect(&mut self, silent: bool) -> Result<(), PeerConnectionError> {
        if !silent {
            self.notify_event(ConnectionManagerEvent::PeerDisconnected(self.peer_node_id.clone()))
                .await;
        }

        self.control.close().await?;

        debug!(
            target: LOG_TARGET,
            "(Peer = {}) Connection closed",
            self.peer_node_id.short_str()
        );

        Ok(())
    }
}

impl fmt::Display for PeerConnectionActor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "PeerConnection(id={}, peer_node_id={}, direction={})",
            self.id,
            self.peer_node_id.short_str(),
            self.direction,
        )
    }
}

pub struct NegotiatedSubstream<TSubstream> {
    pub protocol: ProtocolId,
    pub stream: TSubstream,
}

impl<TSubstream> NegotiatedSubstream<TSubstream> {
    pub fn new(protocol: ProtocolId, stream: TSubstream) -> Self {
        Self { protocol, stream }
    }
}

impl<TSubstream> fmt::Debug for NegotiatedSubstream<TSubstream> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NegotiatedSubstream")
            .field("protocol", &format!("{:?}", self.protocol))
            .field("stream", &"...".to_string())
            .finish()
    }
}
