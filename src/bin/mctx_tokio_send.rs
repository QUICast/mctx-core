use mctx_core::{Context, PublicationConfig, TokioPublication};
use std::env;
use std::error::Error;
use std::net::Ipv4Addr;
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let mut args = env::args().skip(1);

    let group: Ipv4Addr = args.next().ok_or("missing multicast group")?.parse()?;
    let port: u16 = args.next().ok_or("missing destination port")?.parse()?;
    let payload = args.next().ok_or("missing payload")?;
    let count: u64 = args
        .next()
        .map(|value| value.parse())
        .transpose()?
        .unwrap_or(1);
    let interval_ms: u64 = args
        .next()
        .map(|value| value.parse())
        .transpose()?
        .unwrap_or(0);

    let mut context = Context::new();
    let id = context.add_publication(PublicationConfig::new(group, port))?;
    let publication = context.take_publication(id).unwrap();
    let publication = TokioPublication::new(publication)?;
    let interval = Duration::from_millis(interval_ms);

    for _ in 0..count {
        let report = publication.send(payload.as_bytes()).await?;
        println!("sent {} bytes to {}", report.bytes_sent, report.destination);

        if !interval.is_zero() {
            tokio::time::sleep(interval).await;
        }
    }

    Ok(())
}
