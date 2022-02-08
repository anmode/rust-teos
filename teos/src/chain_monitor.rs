//! Logic related to the ChainMonitor, the component in charge of querying block data from `bitcoind`.
//!

use std::ops::Deref;
use std::sync::{Arc, Mutex};
use std::time;
use tokio::time::timeout;
use triggered::Listener;

use lightning::chain;
use lightning_block_sync::poll::{ChainTip, Poll, ValidatedBlockHeader};
use lightning_block_sync::{Cache, SpvClient};

use crate::dbm::DBM;

/// Component in charge of monitoring the chain for new blocks.
///
/// Takes care of polling `bitcoind` for new tips and hand it to subscribers.
/// It is mainly a wrapper around [chain::Listen] that provides some logging.
pub struct ChainMonitor<'a, P, C, L>
where
    P: Poll,
    C: Cache,
    L: Deref,
    L::Target: chain::Listen,
{
    /// A bitcoin client to poll best tips from.
    spv_client: SpvClient<'a, P, C, L>,
    /// The lat known block header by the [ChainMonitor].
    last_known_block_header: ValidatedBlockHeader,
    /// A [DBM] (database manager) instance. Used to persist block data into disk.
    dbm: Arc<Mutex<DBM>>,
    /// The time between polls.
    polling_delta: time::Duration,
    /// A signal from the main thread indicating the tower is shuting down.
    shutdown_signal: Listener,
}

impl<'a, P, C, L> ChainMonitor<'a, P, C, L>
where
    P: Poll,
    C: Cache,
    L: Deref,
    L::Target: chain::Listen,
{
    /// Creates a new [ChainMonitor] instance.
    pub async fn new(
        spv_client: SpvClient<'a, P, C, L>,
        last_known_block_header: ValidatedBlockHeader,
        dbm: Arc<Mutex<DBM>>,
        polling_delta_sec: u64,
        shutdown_signal: Listener,
    ) -> ChainMonitor<'a, P, C, L> {
        ChainMonitor {
            spv_client,
            last_known_block_header,
            dbm,
            polling_delta: time::Duration::from_secs(polling_delta_sec),
            shutdown_signal,
        }
    }

    /// Polls the best chain tip from bitcoind. Serves the data to its listeners (through [chain::Listen]) and logs data about the polled tips.
    pub async fn poll_best_tip(&mut self) {
        match self.spv_client.poll_best_tip().await {
            Ok((chain_tip, _)) => match chain_tip {
                ChainTip::Common => log::debug!("No new best tip found"),

                ChainTip::Better(new_best) => {
                    log::debug!("Updating best tip: {}", new_best.header.block_hash());
                    self.last_known_block_header = new_best;
                    self.dbm
                        .lock()
                        .unwrap()
                        .store_last_known_block(&new_best.header.block_hash())
                        .unwrap();
                }
                ChainTip::Worse(worse) => {
                    // This would happen both if a block has less chainwork than the previous one, or if it has the same chainwork
                    // but it forks from the parent. In both cases, it'll be detected as a reorg once (if) the new chain grows past
                    // the current tip.
                    log::warn!("Worse tip found: {:?}", worse.header.block_hash());

                    if worse.chainwork == self.last_known_block_header.chainwork {
                        log::warn!("New tip has the same work as the previous one")
                    } else {
                        log::warn!("New tip has less work than the previous one")
                    }
                }
            },
            // FIXME: This may need finer catching
            Err(_) => log::error!("Connection lost with bitcoind"),
        };
    }

    /// Monitors `bitcoind` polling the best chain tip every [polling_delta](Self::polling_delta).
    pub async fn monitor_chain(&mut self) {
        loop {
            self.poll_best_tip().await;
            // Sleep for self.polling_delta seconds or shutdown if the signal is received.
            if timeout(self.polling_delta, self.shutdown_signal.clone())
                .await
                .is_ok()
            {
                log::debug!("Received shutting down signal. Shutting down");
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::HashSet;
    use std::iter::FromIterator;

    use bitcoin::network::constants::Network;
    use bitcoin::BlockHash;
    use lightning_block_sync::{poll::ChainPoller, SpvClient, UnboundedCache};

    use crate::test_utils::{Blockchain, START_HEIGHT};

    pub(crate) struct DummyListener {
        pub connected_blocks: RefCell<HashSet<BlockHash>>,
        pub disconnected_blocks: RefCell<HashSet<BlockHash>>,
    }

    impl DummyListener {
        fn new() -> Self {
            Self {
                connected_blocks: RefCell::new(HashSet::new()),
                disconnected_blocks: RefCell::new(HashSet::new()),
            }
        }
    }

    impl chain::Listen for DummyListener {
        fn block_connected(&self, block: &bitcoin::Block, _: u32) {
            self.connected_blocks
                .borrow_mut()
                .insert(block.block_hash());
        }

        fn block_disconnected(&self, header: &bitcoin::BlockHeader, _: u32) {
            self.disconnected_blocks
                .borrow_mut()
                .insert(header.block_hash());
        }
    }

    #[tokio::test]
    async fn test_poll_best_tip_common() {
        let mut chain = Blockchain::default().with_height_and_txs(START_HEIGHT, None);
        let tip = chain.tip();

        let dbm = Arc::new(Mutex::new(DBM::in_memory().unwrap()));
        let (_, shutdown_signal) = triggered::trigger();
        let listener = DummyListener::new();

        let poller = ChainPoller::new(&mut chain, Network::Bitcoin);
        let cache = &mut UnboundedCache::new();
        let spv_client = SpvClient::new(tip, poller, cache, &listener);

        let mut cm = ChainMonitor::new(spv_client, tip, dbm, 1, shutdown_signal).await;

        // If there's no new block nothing gets connected nor disconnected
        cm.poll_best_tip().await;
        assert!(listener.connected_blocks.borrow().is_empty());
        assert!(listener.disconnected_blocks.borrow().is_empty());
    }

    #[tokio::test]
    async fn test_poll_best_tip_better() {
        let mut chain = Blockchain::default().with_height_and_txs(START_HEIGHT, None);
        let new_tip = chain.tip();
        let old_tip = chain.at_height(START_HEIGHT - 1);

        let dbm = Arc::new(Mutex::new(DBM::in_memory().unwrap()));
        let (_, shutdown_signal) = triggered::trigger();
        let listener = DummyListener::new();

        let poller = ChainPoller::new(&mut chain, Network::Bitcoin);
        let cache = &mut UnboundedCache::new();
        let spv_client = SpvClient::new(old_tip, poller, cache, &listener);

        let mut cm = ChainMonitor::new(spv_client, old_tip, dbm, 1, shutdown_signal).await;

        // If a new (best) block gets mined, it should be connected
        cm.poll_best_tip().await;
        assert_eq!(cm.last_known_block_header, new_tip);
        assert_eq!(
            cm.dbm.lock().unwrap().load_last_known_block().unwrap(),
            new_tip.deref().header.block_hash()
        );
        assert!(listener
            .connected_blocks
            .borrow()
            .contains(&new_tip.deref().header.block_hash()));
        assert!(listener.disconnected_blocks.borrow().is_empty());
    }

    #[tokio::test]
    async fn test_poll_best_tip_worse() {
        let mut chain = Blockchain::default().with_height_and_txs(START_HEIGHT, None);
        let best_tip = chain.tip();
        chain.disconnect_tip();

        let dbm = Arc::new(Mutex::new(DBM::in_memory().unwrap()));
        let (_, shutdown_signal) = triggered::trigger();
        let listener = DummyListener::new();

        let poller = ChainPoller::new(&mut chain, Network::Bitcoin);
        let cache = &mut UnboundedCache::new();
        let spv_client = SpvClient::new(best_tip, poller, cache, &listener);

        let mut cm = ChainMonitor::new(spv_client, best_tip, dbm, 1, shutdown_signal).await;

        // If a new (worse, just one) block gets mined, nothing gets connected nor disconnected
        cm.poll_best_tip().await;
        assert_eq!(cm.last_known_block_header, best_tip);
        assert!(matches!(
            cm.dbm.lock().unwrap().load_last_known_block(),
            Err { .. }
        ));
        assert!(listener.connected_blocks.borrow().is_empty());
        assert!(listener.disconnected_blocks.borrow().is_empty());
    }

    #[tokio::test]
    async fn test_poll_best_tip_reorg() {
        let mut chain = Blockchain::default().with_height_and_txs(START_HEIGHT, None);
        let old_best = chain.tip();
        // Reorg
        chain.disconnect_tip();
        let new_blocks = (0..2)
            .map(|_| chain.generate(None).block_hash())
            .collect::<HashSet<BlockHash>>();

        let new_best = chain.tip();

        let dbm = Arc::new(Mutex::new(DBM::in_memory().unwrap()));
        let (_, shutdown_signal) = triggered::trigger();
        let listener = DummyListener::new();

        let poller = ChainPoller::new(&mut chain, Network::Bitcoin);
        let cache = &mut UnboundedCache::new();
        let spv_client = SpvClient::new(old_best, poller, cache, &listener);

        let mut cm = ChainMonitor::new(spv_client, old_best, dbm, 1, shutdown_signal).await;

        // If a a reorg is found (tip is disconnected and a new best is found), both data should be connected and disconnected
        cm.poll_best_tip().await;
        assert_eq!(cm.last_known_block_header, new_best);
        assert_eq!(
            cm.dbm.lock().unwrap().load_last_known_block().unwrap(),
            new_best.deref().header.block_hash()
        );
        assert_eq!(*listener.connected_blocks.borrow(), new_blocks);
        assert_eq!(
            *listener.disconnected_blocks.borrow(),
            HashSet::from_iter([old_best.deref().header.block_hash()])
        );
    }
}
