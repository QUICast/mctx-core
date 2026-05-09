#[path = "common/send_args.rs"]
mod send_args;

use mctx_core::Context;
use std::env;
use std::error::Error;
use std::thread;
use std::time::Duration;

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = env::args().collect();
    let parsed = match send_args::parse_send_cli_args(&args) {
        Ok(parsed) => parsed,
        Err(err) => {
            send_args::print_usage(&args[0]);
            return Err(err.into());
        }
    };

    let mut context = Context::new();
    let id = context.add_publication(parsed.build_config()?)?;
    let interval = Duration::from_millis(parsed.interval_ms);

    for _ in 0..parsed.count {
        let report = context.send(id, parsed.payload.as_bytes())?;
        println!(
            "sent {} bytes to {} from {:?}",
            report.bytes_sent, report.destination, report.source_addr
        );

        if !interval.is_zero() {
            thread::sleep(interval);
        }
    }

    Ok(())
}
