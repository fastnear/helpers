use dotenv::dotenv;
use fastnear_neardata_fetcher::fetcher::{fetch_block_until_success, DEFAULT_TIMEOUT};
use fastnear_primitives::block_with_tx_hash::BlockWithTxHashes;
use fastnear_primitives::near_indexer_primitives::CryptoHash;
use fastnear_primitives::near_primitives::types::BlockHeight;
use reqwest::Client;
use std::env;
use std::io::Write;
use std::time::Duration;

const URL: &str = "http://localhost:3005";
const TX_FILE_LIMIT: usize = 1_000_000;

fn target_url(suffix: &str) -> String {
    format!("{}{}", URL, suffix)
}

pub async fn fetch_first_block(client: &Client) -> Option<BlockWithTxHashes> {
    fetch_block_until_success(client, &target_url("/v0/first_block"), DEFAULT_TIMEOUT).await
}

pub async fn fetch_last_block(client: &Client) -> Option<BlockWithTxHashes> {
    fetch_block_until_success(client, &target_url("/v0/last_block/final"), DEFAULT_TIMEOUT).await
}

pub async fn fetch_block_by_height(
    client: &Client,
    height: BlockHeight,
    timeout: Duration,
) -> Option<BlockWithTxHashes> {
    fetch_block_until_success(
        client,
        &target_url(&format!("/v0/block/{}", height)),
        timeout,
    )
    .await
}

pub fn write_tx_hashes(
    tx_hashes: &[CryptoHash],
    tx_block_height_start: BlockHeight,
    tx_block_height_end: BlockHeight,
) {
    // Padded with up to 9 zeros
    let filename = format!(
        "res/hashes/tx_{:09}_{:09}.txt",
        tx_block_height_start, tx_block_height_end
    );
    // Create new file for writing
    let mut f = std::fs::File::create(&filename).expect("Unable to create file");
    for tx_hash in tx_hashes {
        writeln!(f, "{}", tx_hash).expect("Unable to write data");
    }
    f.flush().expect("Unable to flush file");
    println!("Saved tx file: {}", filename);
}

#[tokio::main]
async fn main() {
    openssl_probe::init_ssl_cert_env_vars();
    dotenv().ok();

    // Check if res exists
    std::fs::create_dir_all("res/hashes").expect("Unable to create directory");

    let args: Vec<String> = env::args().collect();

    let starting_block_height = args
        .get(1)
        .map(|arg| arg.parse().expect("Invalid block height"));

    let client = Client::new();
    let first_block_height = fetch_first_block(&client)
        .await
        .expect("First block doesn't exists")
        .block
        .header
        .height;

    let starting_block_height = starting_block_height
        .unwrap_or(first_block_height)
        .max(first_block_height);

    let last_block_height = fetch_last_block(&client)
        .await
        .expect("Last block doesn't exists")
        .block
        .header
        .height;

    let last_block_height = args
        .get(2)
        .map(|arg| arg.parse().expect("Invalid block height"))
        .unwrap_or(last_block_height);

    let mut prev_block_hash = None;
    let log_file = format!(
        "/tmp/tx_extractor_neardata_{}_{}.log",
        starting_block_height, last_block_height
    );
    let mut f = std::fs::File::create(&log_file).expect("Unable to create file");
    println!("Created log file: {}", log_file);

    let mut tx_hashes = vec![];
    let mut tx_block_height_start = starting_block_height;

    for block_height in starting_block_height..=last_block_height {
        let block = fetch_block_by_height(&client, block_height, DEFAULT_TIMEOUT).await;
        if block.is_none() {
            println!("Block doesn't exist at height: {}", block_height);
            continue;
        }
        let block = block.unwrap();

        assert_eq!(
            block_height, block.block.header.height,
            "Block heights don't match at block height: {}",
            block_height
        );
        println!("Processing block: {}", block_height);
        let block_hash = block.block.header.hash.clone();
        if let Some(prev_block_hash) = prev_block_hash {
            assert_eq!(
                prev_block_hash, block.block.header.prev_hash,
                "Block hashes don't match at block height: {}",
                block.block.header.height
            );
        }
        for shard in block.shards {
            if let Some(chunk) = shard.chunk {
                for tx in chunk.transactions {
                    tx_hashes.push(tx.transaction.hash);
                }
            }
        }
        if tx_hashes.len() >= TX_FILE_LIMIT {
            write_tx_hashes(&tx_hashes, tx_block_height_start, block_height);
            tx_hashes = vec![];
            tx_block_height_start = block_height + 1;
        }

        prev_block_hash = Some(block_hash);
    }
    println!("All blocks are executed");
    if tx_hashes.len() >= TX_FILE_LIMIT {
        write_tx_hashes(&tx_hashes, tx_block_height_start, last_block_height);
    }

    f.flush().expect("Unable to flush file");
    println!("Saved log file: {}", log_file);
}
