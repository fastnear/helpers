use dotenv::dotenv;
use fastnear_neardata_fetcher::{fetcher, FetcherConfig};
use fastnear_primitives::types::ChainId;
use std::env;
use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::mpsc;

#[tokio::main]
async fn main() {
    openssl_probe::init_ssl_cert_env_vars();
    dotenv().ok();

    let args: Vec<String> = env::args().collect();

    let start_block_height = args
        .get(1)
        .map(|arg| arg.parse().expect("Invalid block height"))
        .expect("Start block height is required");

    let end_block_height = args
        .get(2)
        .map(|arg| arg.parse().expect("Invalid block height"))
        .expect("End block height is required");

    let num_threads = env::var("NUM_THREADS")
        .ok()
        .map(|num_threads| num_threads.parse().expect("Invalid number of threads"))
        .unwrap_or(8);

    let auth_bearer_token = env::var("FASTNEAR_AUTH_BEARER_TOKEN").ok();

    let filename = format!(
        "/tmp/validators_{}_{}.csv",
        start_block_height, end_block_height
    );
    let mut f = std::fs::File::create(&filename).expect("Unable to create file");
    println!("Created file: {}", filename);

    let is_running = Arc::new(AtomicBool::new(true));
    let ctrl_c_running = is_running.clone();

    ctrlc::set_handler(move || {
        ctrl_c_running.store(false, Ordering::SeqCst);
        println!("Received Ctrl+C, starting shutdown...");
    })
    .expect("Error setting Ctrl+C handler");

    let (sender, mut receiver) = mpsc::channel(320);
    tokio::spawn(fetcher::start_fetcher(
        None,
        FetcherConfig {
            num_threads,
            start_block_height,
            chain_id: ChainId::Mainnet,
            timeout_duration: None,
            retry_duration: None,
            disable_archive_sync: false,
            auth_bearer_token,
        },
        sender,
        is_running.clone(),
    ));

    writeln!(f, "block_height,validator_id,shard_id").expect("Unable to write to file");

    while let Some(block) = receiver.recv().await {
        let block_height = block.block.header.height;
        if block_height > end_block_height {
            is_running.store(false, Ordering::SeqCst);
            break;
        }
        if block_height % 100 == 0 {
            println!("Processing block {}", block_height);
        }
        writeln!(f, "{},{},{}", block_height, block.block.author, "block")
            .expect("Unable to write to file");
        for shard in block.shards {
            if let Some(chunk) = shard.chunk {
                writeln!(
                    f,
                    "{},{},{}",
                    block_height, chunk.author, chunk.header.shard_id
                )
                .expect("Unable to write to file");
            }
        }
    }

    f.flush().expect("Unable to flush file");
    println!("Saved log file: {}", filename);
}
