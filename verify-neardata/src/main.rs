use dotenv::dotenv;
use fastnear_neardata_fetcher::fetcher::{fetch_block_until_success, DEFAULT_TIMEOUT};
use fastnear_primitives::block_with_tx_hash::BlockWithTxHashes;
use fastnear_primitives::near_indexer_primitives::views::ReceiptEnumView;
use fastnear_primitives::near_primitives::types::BlockHeight;
use reqwest::Client;
use std::collections::HashMap;
use std::env;
use std::io::Write;
use std::time::Duration;

const URL: &str = "http://localhost:3005";

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

#[tokio::main]
async fn main() {
    openssl_probe::init_ssl_cert_env_vars();
    dotenv().ok();

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

    let mut data_ids = HashMap::new();
    let mut prev_block_hash = None;
    let log_file = format!(
        "/tmp/verify_neardata_{}_{}.log",
        starting_block_height, last_block_height
    );
    let mut f = std::fs::File::create(&log_file).expect("Unable to create file");
    println!("Created file: {}", log_file);

    std::fs::create_dir_all("res").expect("Unable to create directory");
    let mut f_bad_shard_id_blocks =
        std::fs::File::create("res/bad_shard_id_blocks.txt").expect("Unable to create file");

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
        let mut bad_shard_id = false;
        for shard in block.shards {
            if let Some(chunk) = shard.chunk {
                if !bad_shard_id && shard.shard_id != chunk.header.shard_id {
                    bad_shard_id = true;
                    let s = format!(
                        "Bad shard id at block height {}. Shard id: {} Chunk shard id: {}",
                        block_height, shard.shard_id, chunk.header.shard_id
                    );
                    eprintln!("{}", s);
                    f.write(s.as_bytes()).expect("Unable to write to file");
                    f.write(b"\n").expect("Unable to write to file");

                    f_bad_shard_id_blocks
                        .write(format!("{}\n", block_height).as_bytes())
                        .expect("Unable to write to bad shard id file");
                }
                for receipt in chunk.receipts {
                    match receipt.receipt {
                        ReceiptEnumView::Action { .. } => {
                            // skipping here, since we'll get one with execution
                        }
                        ReceiptEnumView::Data {
                            data_id,
                            data,
                            is_promise_resume,
                        } => {
                            if let Some((other_receipt_id, other_data)) =
                                data_ids.insert(data_id, (receipt.receipt_id, data))
                            {
                                let (new_receipt_id, new_data) = data_ids.get(&data_id).unwrap();
                                let s = format!(
                                    "Data id {} is already present at block height {} Receipt ids: {} and {} Data matching: {:?}. Is promise resume: {}",
                                    data_id, block_height, new_receipt_id, other_receipt_id, new_data == &other_data, is_promise_resume
                                );
                                eprintln!("{}", s);
                                f.write(s.as_bytes()).expect("Unable to write to file");
                                f.write(b"\n").expect("Unable to write to file");
                            }
                        }
                    }
                }
            }
            for outcome in shard.receipt_execution_outcomes {
                if outcome.tx_hash.is_none() {
                    panic!(
                        "Tx hash is none at block height {} for receipt id {}",
                        block_height, outcome.receipt.receipt_id
                    );
                }
            }
        }

        prev_block_hash = Some(block_hash);
    }
    println!("All blocks are verified");
    f.flush().expect("Unable to flush file");
    f_bad_shard_id_blocks.flush().expect("Unable to flush file");
    println!("Saved log file: {}", log_file);
}
