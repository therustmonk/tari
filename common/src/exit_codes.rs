use std::fmt;
use thiserror::Error;

#[derive(Debug, Clone, Error)]
pub struct ExitError {
    exit_code: ExitCode,
    details: Option<String>,
}

impl ExitError {
    pub fn new(exit_code: ExitCode, details: Option<String>) -> Self {
        Self { exit_code, details }
    }
}

impl fmt::Display for ExitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let details = self.details.as_ref().map(String::as_ref).unwrap_or("");
        write!(f, "{} {}", self.exit_code, details)
    }
}

impl From<ExitCodes> for ExitError {
    fn from(codes: ExitCodes) -> Self {
        use ExitCodes::*;
        match codes {
            ConfigError(s) => Self::new(ExitCode::ConfigError, Some(s)),
            UnknownError(s) => Self::new(ExitCode::UnknownError, Some(s)),
            InterfaceError => Self::new(ExitCode::InterfaceError, None),
            WalletError(s) => Self::new(ExitCode::WalletError, Some(s)),
            GrpcError(s) => Self::new(ExitCode::GrpcError, Some(s)),
            InputError(s) => Self::new(ExitCode::InputError, Some(s)),
            CommandError(s) => Self::new(ExitCode::CommandError, Some(s)),
            IOError(s) => Self::new(ExitCode::IOError, Some(s)),
            RecoveryError(s) => Self::new(ExitCode::RecoveryError, Some(s)),
            NetworkError(s) => Self::new(ExitCode::NetworkError, Some(s)),
            ConversionError(s) => Self::new(ExitCode::ConversionError, Some(s)),
            IncorrectPassword => Self::new(ExitCode::IncorrectOrEmptyPassword, None),
            NoPassword => Self::new(ExitCode::IncorrectOrEmptyPassword, None),
            TorOffline => Self::new(ExitCode::TorOffline, None),
            DatabaseError(s) => Self::new(ExitCode::DatabaseError, Some(s)),
            DbInconsistentState(s) => Self::new(ExitCode::DbInconsistentState, Some(s)),
        }
    }
}

const TOR_HINT: &str = r#"\
Unable to connect to the Tor control port.

Please check that you have the Tor proxy running and \
that access to the Tor control port is turned on.

If you are unsure of what to do, use the following \
command to start the Tor proxy:
tor --allow-missing-torrc --ignore-missing-torrc \
--clientonly 1 --socksport 9050 --controlport \
127.0.0.1:9051 --log \"warn stdout\" --clientuseipv6 1
"#;

impl ExitCode {
    pub fn hint(&self) -> &str {
        use ExitCode::*;
        match self {
            TorOffline => TOR_HINT,
            _ => "",
        }
    }
}

/// Enum to show failure information
#[derive(Debug, Clone, Error)]
pub enum ExitCode {
    #[error("There is an error in the configuration.")]
    ConfigError = 101,
    #[error("The application exited because an unknown error occurred. Check the logs for more details.")]
    UnknownError = 102,
    #[error("The application exited because an interface error occurred. Check the logs for details.")]
    InterfaceError = 103,
    #[error("The application exited.")]
    WalletError = 104,
    #[error("The application was not able to start the GRPC server.")]
    GrpcError = 105,
    #[error("The application did not accept the command input.")]
    InputError = 106,
    #[error("Invalid command.")]
    CommandError = 107,
    #[error("IO error.")]
    IOError = 108,
    #[error("Recovery failed.")]
    RecoveryError = 109,
    #[error("The application exited because of an internal network error.")]
    NetworkError = 110,
    #[error("The application exited because it received a message it could not interpret.")]
    ConversionError = 111,
    #[error("Your password was incorrect or required, but not provided.")]
    IncorrectOrEmptyPassword = 112,
    #[error("Tor connection is offline")]
    TorOffline = 113,
    #[error("The application encountered a database error.")]
    DatabaseError = 114,
    #[error("Database is in an inconsistent state!")]
    DbInconsistentState = 115,
}

/// Enum to show failure information
#[derive(Debug, Clone, Error)]
pub enum ExitCodes {
    #[error("There is an error in the configuration: {0}")]
    ConfigError(String),
    #[error("The application exited because an unknown error occurred: {0}. Check the logs for more details.")]
    UnknownError(String),
    #[error("The application exited because an interface error occurred. Check the logs for details.")]
    InterfaceError,
    #[error("The application exited. {0}")]
    WalletError(String),
    #[error("The application was not able to start the GRPC server. {0}")]
    GrpcError(String),
    #[error("The application did not accept the command input: {0}")]
    InputError(String),
    #[error("Invalid command: {0}")]
    CommandError(String),
    #[error("IO error: {0}")]
    IOError(String),
    #[error("Recovery failed: {0}")]
    RecoveryError(String),
    #[error("The application exited because of an internal network error: {0}")]
    NetworkError(String),
    #[error("The application exited because it received a message it could not interpret: {0}")]
    ConversionError(String),
    #[error("Your password was incorrect.")]
    IncorrectPassword,
    #[error("Your wallet is encrypted but no password was provided.")]
    NoPassword,
    #[error("The application encountered a database error: {0}")]
    DatabaseError(String),
    #[error("Tor connection is offline")]
    TorOffline,
    #[error("Database is in an inconsistent state!: {0}")]
    DbInconsistentState(String),
}

impl ExitCodes {
    pub fn as_i32(&self) -> i32 {
        match self {
            Self::ConfigError(_) => 101,
            Self::UnknownError(_) => 102,
            Self::InterfaceError => 103,
            Self::WalletError(_) => 104,
            Self::GrpcError(_) => 105,
            Self::InputError(_) => 106,
            Self::CommandError(_) => 107,
            Self::IOError(_) => 108,
            Self::RecoveryError(_) => 109,
            Self::NetworkError(_) => 110,
            Self::ConversionError(_) => 111,
            Self::IncorrectPassword | Self::NoPassword => 112,
            Self::TorOffline => 113,
            Self::DatabaseError(_) => 114,
            Self::DbInconsistentState(_) => 115,
        }
    }

    pub fn eprint_details(&self) {
        use ExitCodes::*;
        match self {
            TorOffline => {
                eprintln!("Unable to connect to the Tor control port.");
                eprintln!(
                    "Please check that you have the Tor proxy running and that access to the Tor control port is \
                     turned on.",
                );
                eprintln!("If you are unsure of what to do, use the following command to start the Tor proxy:");
                eprintln!(
                    "tor --allow-missing-torrc --ignore-missing-torrc --clientonly 1 --socksport 9050 --controlport \
                     127.0.0.1:9051 --log \"warn stdout\" --clientuseipv6 1",
                );
            },
            e => {
                eprintln!("{}", e);
            },
        }
    }
}

impl From<super::ConfigError> for ExitCodes {
    fn from(err: super::ConfigError) -> Self {
        // TODO: Move it out
        // error!(target: LOG_TARGET, "{}", err);
        Self::ConfigError(err.to_string())
    }
}

impl From<crate::ConfigurationError> for ExitCodes {
    fn from(err: crate::ConfigurationError) -> Self {
        Self::ConfigError(err.to_string())
    }
}

impl From<multiaddr::Error> for ExitCodes {
    fn from(err: multiaddr::Error) -> Self {
        Self::ConfigError(err.to_string())
    }
}

impl From<std::io::Error> for ExitCodes {
    fn from(err: std::io::Error) -> Self {
        Self::IOError(err.to_string())
    }
}

impl ExitCodes {
    pub fn grpc<M: std::fmt::Display>(err: M) -> Self {
        ExitCodes::GrpcError(format!("GRPC connection error: {}", err))
    }
}
