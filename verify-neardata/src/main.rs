use dotenv::dotenv;
use fastnear_neardata_fetcher::fetcher::{fetch_block_until_success, DEFAULT_TIMEOUT};
use fastnear_primitives::block_with_tx_hash::BlockWithTxHashes;
use fastnear_primitives::near_indexer_primitives::views::ReceiptEnumView;
use fastnear_primitives::near_primitives::types::BlockHeight;
use reqwest::Client;
use std::collections::HashMap;
use std::env;
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

    let mut data_ids = HashMap::new();
    let mut prev_block_hash = None;
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
                for receipt in chunk.receipts {
                    match receipt.receipt {
                        ReceiptEnumView::Action { .. } => {
                            // skipping here, since we'll get one with execution
                        }
                        ReceiptEnumView::Data { data_id, .. } => {
                            if let Some(other_id) =
                                data_ids.insert(data_id, receipt.receipt_id.clone())
                            {
                                panic!(
                                    "Data id {} is already present at block height {}. Receipt ids: {} and {}",
                                    data_id, block_height, receipt.receipt_id, other_id
                                );
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
}
