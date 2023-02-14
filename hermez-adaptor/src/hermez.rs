#![cfg(any(test, feature = "testing"))]

use async_std::task::sleep;
use contract_bindings::TestHermezContracts;
use ethers::prelude::*;
use portpicker::pick_unused_port;
use std::{
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicUsize, Ordering},
    time::Duration,
};
use surf_disco::Url;

pub async fn wait_for_http(
    url: &Url,
    interval: Duration,
    max_retries: usize,
) -> Result<usize, String> {
    for i in 0..(max_retries + 1) {
        let res = surf::get(url).await;
        if res.is_ok() {
            tracing::debug!("Connected to {url}");
            return Ok(i);
        }
        tracing::debug!("Waiting for {url}, retrying in {interval:?}");
        sleep(interval).await;
    }
    Err(format!("Url {url:?} not available."))
}

pub async fn wait_for_rpc(
    url: &Url,
    interval: Duration,
    max_retries: usize,
) -> Result<usize, String> {
    let retries = wait_for_http(url, interval, max_retries).await?;
    let client = Provider::new(Http::new(url.clone()));
    for i in retries..(max_retries + 1) {
        if client.get_block_number().await.is_ok() {
            tracing::debug!("JSON-RPC ready at {url}");
            return Ok(i);
        }
        tracing::debug!("Waiting for JSON-RPC at {url}, retrying in {interval:?}");
        sleep(interval).await;
    }
    Err(format!("No JSON-RPC at {url}"))
}

#[derive(Clone, Debug)]
pub struct ZkEvmEnv {
    cdn_server_port: u16,
    sequencer_api_port: u16,
    sequencer_storage_path: PathBuf,
    l1_port: u16,
    l2_port: u16,
    l1_chain_id: Option<u64>,
    l2_chain_id: Option<u64>,
    sequencer_mnemonic: String,
    adaptor_port: u16,
}

impl ZkEvmEnv {
    pub fn random() -> Self {
        let cdn_server_port = pick_unused_port().unwrap();
        let sequencer_api_port = pick_unused_port().unwrap();
        let l1_port = pick_unused_port().unwrap();
        let l2_port = pick_unused_port().unwrap();
        let adaptor_port = pick_unused_port().unwrap();

        // Use default values for things that are deterministic or internal to a docker-compose
        // service.
        let sequencer_storage_path = "/store/sequencer".into();
        let sequencer_mnemonic =
            "test test test test test test test test test test test junk".into();
        let l1_chain_id = None;
        let l2_chain_id = None;

        Self {
            cdn_server_port,
            sequencer_api_port,
            l1_port,
            l2_port,
            l1_chain_id,
            l2_chain_id,
            adaptor_port,
            sequencer_storage_path,
            sequencer_mnemonic,
        }
    }

    pub fn cmd(&self, command: &str) -> Command {
        let mut cmd = Command::new(command);
        cmd.env("ESPRESSO_CDN_SERVER_PORT", self.cdn_server_port.to_string())
            .env(
                "ESPRESSO_SEQUENCER_API_PORT",
                self.sequencer_api_port.to_string(),
            )
            .env("ESPRESSO_SEQUENCER_URL", self.sequencer().as_ref())
            .env(
                "ESPRESSO_SEQUENCER_STORAGE_PATH",
                self.sequencer_storage_path.as_os_str(),
            )
            .env("ESPRESSO_ZKEVM_L1_PORT", self.l1_port.to_string())
            .env("ESPRESSO_ZKEVM_L1_PROVIDER", self.l1_provider().as_ref())
            .env("ESPRESSO_ZKEVM_L2_PORT", self.l2_port.to_string())
            .env("ESPRESSO_ZKEVM_L2_PROVIDER", self.l2_provider().as_ref())
            .env(
                "ESPRESSO_ZKEVM_SEQUENCER_MNEMONIC",
                &self.sequencer_mnemonic,
            )
            .env("ESPRESSO_ZKEVM_ADAPTOR_PORT", self.adaptor_port.to_string())
            .env(
                "ESPRESSO_ZKEVM_ADAPTOR_URL",
                format!("http://host.docker.internal:{}", self.adaptor_port),
            );
        if let Some(id) = self.l1_chain_id {
            cmd.env("ESPRESSO_ZKEVM_L1_CHAIN_ID", id.to_string());
        }
        if let Some(id) = self.l2_chain_id {
            cmd.env("ESPRESSO_ZKEVM_L2_CHAIN_ID", id.to_string());
        }
        cmd
    }

    pub fn l1_provider(&self) -> Url {
        format!("http://localhost:{}", self.l1_port)
            .parse()
            .unwrap()
    }

    pub fn l1_chain_id(&self) -> Option<u64> {
        self.l1_chain_id
    }

    pub fn l2_provider(&self) -> Url {
        format!("http://localhost:{}", self.l2_port)
            .parse()
            .unwrap()
    }

    pub fn l2_chain_id(&self) -> Option<u64> {
        self.l2_chain_id
    }

    pub fn funded_mnemonic(&self) -> &str {
        &self.sequencer_mnemonic
    }

    pub fn sequencer_port(&self) -> u16 {
        self.sequencer_api_port
    }

    pub fn sequencer(&self) -> Url {
        format!("http://localhost:{}", self.sequencer_api_port)
            .parse()
            .unwrap()
    }

    pub fn l2_adaptor_port(&self) -> u16 {
        self.adaptor_port
    }

    pub fn l2_adaptor(&self) -> Url {
        format!("http://localhost:{}", self.adaptor_port)
            .parse()
            .unwrap()
    }
}

/// A zkevm-node inside docker compose with custom contracts
#[derive(Debug, Clone)]
pub struct ZkEvmNode {
    env: ZkEvmEnv,
    l1: TestHermezContracts,
    project_name: String,
}

impl ZkEvmNode {
    pub fn env(&self) -> &ZkEvmEnv {
        &self.env
    }

    pub fn l1(&self) -> &TestHermezContracts {
        &self.l1
    }

    pub fn project_name(&self) -> &String {
        &self.project_name
    }

    fn compose_cmd_prefix(env: &ZkEvmEnv, project_name: &str) -> Command {
        let mut cmd = env.cmd("docker");
        let work_dir = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        cmd.current_dir(work_dir)
            .arg("compose")
            .arg("--project-name")
            .arg(project_name)
            .arg("-f")
            .arg("permissionless-docker-compose.yaml")
            .arg("-f")
            .arg("docker-compose-anvil.yaml");
        cmd
    }

    /// Start the L1, deploy contracts, start the L2
    pub async fn start(project_name: String) -> Self {
        // Add a unique number to `project_name` to ensure that all instances use a unique name.
        static ID_COUNTER: AtomicUsize = AtomicUsize::new(0);
        let project_name = format!(
            "{}-{}",
            project_name,
            ID_COUNTER.fetch_add(1, Ordering::SeqCst)
        );

        let env = ZkEvmEnv::random();
        tracing::info!("Starting ZkEvmNode with env: {:?}", env);
        tracing::info!(
            "Compose prefix: {:?}",
            Self::compose_cmd_prefix(&env, &project_name)
        );

        // Pull Docker images before doing anything. This can take a long time, and if we do it
        // later it might cause some `wait_for_http` calls to time out.
        let status = Self::compose_cmd_prefix(&env, &project_name)
            .arg("pull")
            .spawn()
            .expect("Failed to pull Docker images")
            .wait()
            .expect("Error waiting for Docker pull command");
        if !status.success() {
            panic!("Failed to pull Docker images ({status})");
        }

        // Start L1
        Self::compose_cmd_prefix(&env, &project_name)
            .arg("up")
            .arg("zkevm-mock-l1-network")
            .arg("-V")
            .arg("--force-recreate")
            .arg("--abort-on-container-exit")
            .spawn()
            .expect("Failed to start L1 docker container");

        println!("Waiting for L1 to start ...");

        wait_for_rpc(&env.l1_provider(), Duration::from_millis(200), 100)
            .await
            .unwrap();

        // Use a dummy URL for the trusted sequencer since we're not running one anyways.
        let l1 = TestHermezContracts::deploy(&env.l1_provider(), "http://dummy:1234").await;

        // Start zkevm-node
        Self::compose_cmd_prefix(&env, &project_name)
            .env(
                "ESPRESSO_ZKEVM_ROLLUP_ADDRESS",
                format!("{:?}", l1.rollup.address()),
            )
            .env(
                "ESPRESSO_ZKEVM_MATIC_ADDRESS",
                format!("{:?}", l1.matic.address()),
            )
            .env(
                "ESPRESSO_ZKEVM_GER_ADDRESS",
                format!("{:?}", l1.global_exit_root.address()),
            )
            .arg("up")
            .arg("zkevm-prover")
            .arg("zkevm-aggregator")
            .arg("zkevm-state-db")
            .arg("zkevm-permissionless-node")
            .arg("-V")
            .arg("--force-recreate")
            .arg("--abort-on-container-exit")
            .spawn()
            .expect("Failed to start zkevm-node compose environment");

        wait_for_rpc(&env.l2_provider(), Duration::from_secs(1), 100)
            .await
            .expect("Failed to start zkevm-node");

        Self {
            env,
            project_name,
            l1,
        }
    }

    fn stop(&self) -> &Self {
        Self::compose_cmd_prefix(&self.env, &self.project_name)
            .arg("down")
            .arg("-v")
            .arg("--remove-orphans")
            .spawn()
            .expect("Failed to run docker compose down");
        self
    }
}

impl Drop for ZkEvmNode {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use async_compatibility_layer::logging::{setup_backtrace, setup_logging};

    // This test currently causes an OOM on the GitHub runners, so it is disabled to avoid CI
    // failures.
    #[async_std::test]
    #[ignore]
    async fn test_two_nodes() {
        setup_logging();
        setup_backtrace();

        let node1 = async_std::task::spawn(ZkEvmNode::start("node-1".to_string()));
        let node2 = async_std::task::spawn(ZkEvmNode::start("node-2".to_string()));
        node2.await;
        node1.await;
    }
}