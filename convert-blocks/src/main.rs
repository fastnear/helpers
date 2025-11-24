use dotenv::dotenv;
use fastnear_neardata_fetcher::{fetcher, FetcherConfigBuilder};
use fastnear_primitives::block_with_tx_hash::BlockWithTxHashes;
use fastnear_primitives::near_indexer_primitives::types::BlockHeight;
use fastnear_primitives::types::ChainId;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::{env, fs};
use tokio::sync::mpsc;

#[tokio::main]
async fn main() {
    openssl_probe::init_ssl_cert_env_vars();
    dotenv().ok();

    let args: Vec<String> = env::args().collect();

    let start_block_height: BlockHeight = args
        .get(1)
        .map(|arg| arg.parse().expect("Invalid block height"))
        .expect("Start block height is required");

    let end_block_height: BlockHeight = args
        .get(2)
        .map(|arg| arg.parse().expect("Invalid block height"))
        .expect("End block height is required");

    let num_threads = env::var("NUM_THREADS")
        .ok()
        .map(|num_threads| num_threads.parse().expect("Invalid number of threads"))
        .unwrap_or(8);

    let auth_bearer_token = env::var("FASTNEAR_AUTH_BEARER_TOKEN").ok();

    let nearlake_blocks_path =
        env::var("NEARLAKE_BLOCKS_PATH").expect("Nearlake blocks path is required");

    let save_path = env::var("SAVE_PATH").ok();

    let is_running = Arc::new(AtomicBool::new(true));
    let ctrl_c_running = is_running.clone();

    ctrlc::set_handler(move || {
        ctrl_c_running.store(false, Ordering::SeqCst);
        println!("Received Ctrl+C, starting shutdown...");
    })
    .expect("Error setting Ctrl+C handler");

    let mut config = FetcherConfigBuilder::new()
        .num_threads(num_threads)
        .start_block_height(start_block_height)
        .end_block_height(end_block_height)
        .chain_id(ChainId::Testnet);

    if let Some(token) = auth_bearer_token.clone() {
        config = config.auth_bearer_token(token);
    }

    let (sender, mut receiver) = mpsc::channel(320);
    tokio::spawn(fetcher::start_fetcher(
        config.build(),
        sender,
        is_running.clone(),
    ));

    let mut blocks = vec![];
    let save_every_n = 10;

    while let Some(mut block) = receiver.recv().await {
        let block_height = block.block.header.height;
        if block_height % 100 == 0 {
            println!("Processing block {}", block_height);
        }
        let filename = format!("{}/{:0>12}.csv", nearlake_blocks_path, block_height);
        let content = std::fs::read_to_string(&filename)
            .unwrap_or_else(|_| "block_height,validator_id,shard_id\n".to_string());
        let lake_block: BlockWithTxHashes =
            serde_json::from_str(&content).expect("Unable to parse block file");

        assert_eq!(
            lake_block.block.header.height, block_height,
            "Block height mismatch"
        );
        assert_eq!(
            lake_block.shards.len(),
            block.shards.len(),
            "Shards length mismatch"
        );

        for shard in &mut block.shards {
            let lake_shard = lake_block
                .shards
                .iter()
                .find(|s| s.shard_id == shard.shard_id)
                .expect("Shard not found in lake block");
            assert_eq!(
                shard.state_changes.len(),
                lake_shard.state_changes.len(),
                "State changes length mismatch"
            );
            shard.state_changes = lake_shard.state_changes.clone();
        }

        if blocks.get(0).map(|(height, _block)| *height).unwrap_or(0) / save_every_n
            != block_height / save_every_n
        {
            save_blocks(&blocks, save_path.as_ref(), save_every_n).unwrap();
            blocks.clear();
        }

        let updated_content =
            serde_json::to_string(&block).expect("Unable to serialize updated block");

        blocks.push((block_height, updated_content));
    }

    if !blocks.is_empty() {
        save_blocks(&blocks, save_path.as_ref(), save_every_n).unwrap();
    }
}

fn save_blocks(
    blocks: &[(BlockHeight, String)],
    path: Option<&String>,
    save_every_n: u64,
) -> std::io::Result<()> {
    if blocks.is_empty() || path.is_none() {
        return Ok(());
    }
    let starting_block = blocks[0].0 / save_every_n * save_every_n;
    let padded_block_height = format!("{:0>12}", starting_block);
    let path = format!(
        "{}/{}/{}",
        path.unwrap(),
        &padded_block_height[..6],
        &padded_block_height[6..9]
    );

    fs::create_dir_all(&path)?;
    let filename = format!("{}/{}.tgz", path, padded_block_height);
    println!("Saving blocks to: {}", filename);
    // Creating archive
    let tar_gz = fs::File::create(&filename)?;
    let enc = flate2::write::GzEncoder::new(tar_gz, flate2::Compression::default());
    let mut tar = tar::Builder::new(enc);
    for (block_height, block_str) in blocks {
        let padded_block_height = format!("{:0>12}", block_height);
        let mut header = tar::Header::new_gnu();
        header.set_path(&format!("{}.json", padded_block_height))?;
        header.set_size(block_str.len() as u64);
        header.set_cksum();
        tar.append(&header, block_str.as_bytes())?;
    }

    tar.into_inner()?.finish()?;
    Ok(())
}
