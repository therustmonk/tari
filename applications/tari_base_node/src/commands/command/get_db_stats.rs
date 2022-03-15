use anyhow::Error;
use async_trait::async_trait;
use clap::Parser;

use super::{CommandContext, HandleCommand};
use crate::table::Table;

/// Gets your base node database stats
#[derive(Debug, Parser)]
pub struct Args {}

#[async_trait]
impl HandleCommand<Args> for CommandContext {
    async fn handle_command(&mut self, _: Args) -> Result<(), Error> {
        self.get_blockchain_db_stats().await
    }
}

impl CommandContext {
    pub async fn get_blockchain_db_stats(&self) -> Result<(), Error> {
        const BYTES_PER_MB: usize = 1024 * 1024;

        let stats = self.blockchain_db.get_stats().await?;
        let mut table = Table::new();
        table.set_titles(vec![
            "Name",
            "Entries",
            "Depth",
            "Branch Pages",
            "Leaf Pages",
            "Overflow Pages",
            "Est. Size (MiB)",
            "% of total",
        ]);
        let total_db_size = stats.db_stats().iter().map(|s| s.total_page_size()).sum::<usize>();
        stats.db_stats().iter().for_each(|stat| {
            table.add_row(row![
                stat.name,
                stat.entries,
                stat.depth,
                stat.branch_pages,
                stat.leaf_pages,
                stat.overflow_pages,
                format!("{:.2}", stat.total_page_size() as f32 / BYTES_PER_MB as f32),
                format!("{:.2}%", (stat.total_page_size() as f32 / total_db_size as f32) * 100.0)
            ]);
        });

        table.print_stdout();
        println!();
        println!(
            "{} databases, {:.2} MiB used ({:.2}%), page size: {} bytes, env_info = ({})",
            stats.root().entries,
            total_db_size as f32 / BYTES_PER_MB as f32,
            (total_db_size as f32 / stats.env_info().mapsize as f32) * 100.0,
            stats.root().psize as usize,
            stats.env_info()
        );

        println!();
        println!("Totalling DB entry sizes. This may take a few seconds...");
        println!();
        let stats = self.blockchain_db.fetch_total_size_stats().await?;
        println!();
        let mut table = Table::new();
        table.set_titles(vec![
            "Name",
            "Entries",
            "Total Size (MiB)",
            "Avg. Size/Entry (bytes)",
            "% of total",
        ]);
        let total_data_size = stats.sizes().iter().map(|s| s.total()).sum::<u64>();
        stats.sizes().iter().for_each(|size| {
            let total = size.total() as f32 / BYTES_PER_MB as f32;
            table.add_row(row![
                size.name,
                size.num_entries,
                format!("{:.2}", total),
                format!("{}", size.avg_bytes_per_entry()),
                format!("{:.2}%", (size.total() as f32 / total_data_size as f32) * 100.0)
            ])
        });
        table.print_stdout();
        println!();
        println!(
            "Total blockchain data size: {:.2} MiB ({:.2} % of LMDB map size)",
            total_data_size as f32 / BYTES_PER_MB as f32,
            (total_data_size as f32 / total_db_size as f32) * 100.0
        );
        Ok(())
    }
}
