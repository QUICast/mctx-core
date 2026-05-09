#[path = "common/send_args.rs"]
mod send_args;

use mctx_core::{Context, TokioPublication};
use std::env;
use std::error::Error;
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = env::args().collect();
    let parsed = match send_args::parse_send_cli_args(&args) {
        Ok(parsed) => parsed,
        Err(err) => {
            send_args::print_usage(&args[0], false);
            return Err(err.into());
        }
    };

    let mut context = Context::new();
    let id = context.add_publication(parsed.build_config()?)?;
    let publication = context.take_publication(id).unwrap();
    let publication = TokioPublication::new(publication)?;
    let interval = Duration::from_millis(parsed.interval_ms);

    for _ in 0..parsed.count {
        let report = publication.send(parsed.payload.as_bytes()).await?;
        println!(
            "sent {} bytes to {} from {:?}",
            report.bytes_sent, report.destination, report.source_addr
        );

        if !interval.is_zero() {
            tokio::time::sleep(interval).await;
        }
    }

    Ok(())
}
