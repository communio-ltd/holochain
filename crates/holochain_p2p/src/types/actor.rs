//! Module containing the HolochainP2p actor definition.
#![allow(clippy::too_many_arguments)]

use crate::*;

/// Request a validation package.
pub struct GetValidationPackage {
    /// The dna_hash / space_hash context.
    pub dna_hash: DnaHash,
    /// The agent_id / agent_pub_key context.
    pub agent_pub_key: AgentPubKey,
    // TODO - parameters
}

/// Get options help control how the get is processed at various levels.
/// Fields tagged with `[Network]` are network-level controls.
/// Fields tagged with `[Remote]` are controls that will be forwarded to the
/// remote agent processing this `Get` request.
pub struct GetOptions {
    /// [Network]
    /// How many remote nodes should we make requests of / aggregate.
    /// Set to `None` for a default "best-effort".
    pub remote_agent_count: Option<u8>,

    /// [Network]
    /// Timeout to await responses for aggregation.
    /// Set to `None` for a default "best-effort".
    /// Note - if all requests time-out you will receive an empty result,
    /// not a timeout error.
    pub timeout_ms: Option<u64>,

    /// [Network]
    /// We are interested in speed. If `true` and we have any results
    /// when `race_timeout_ms` is expired, those results will be returned.
    /// After `race_timeout_ms` and before `timeout_ms` the first result
    /// received will be returned.
    pub as_race: bool,

    /// [Network]
    /// See `as_race` for details.
    /// Set to `None` for a default "best-effort" race.
    pub race_timeout_ms: Option<u64>,

    /// [Remote]
    /// Whether the remote-end should follow redirects or just return the
    /// requested entry.
    pub follow_redirects: bool,
}

impl Default for GetOptions {
    fn default() -> Self {
        Self {
            remote_agent_count: None,
            timeout_ms: None,
            as_race: true,
            race_timeout_ms: None,
            follow_redirects: true,
        }
    }
}

/// Get links from the DHT.
pub struct GetLinks {
    /// The dna_hash / space_hash context.
    pub dna_hash: DnaHash,
    /// The agent_id / agent_pub_key context.
    pub agent_pub_key: AgentPubKey,
    // TODO - parameters
}

ghost_actor::ghost_actor! {
    /// The HolochainP2pSender struct allows controlling the HolochainP2p
    /// actor instance.
    pub actor HolochainP2p<HolochainP2pError> {
        /// The p2p module must be informed at runtime which dna/agent pairs it should be tracking.
        fn join(dna_hash: DnaHash, agent_pub_key: AgentPubKey) -> ();

        /// If a cell is deactivated, we'll need to \"leave\" the network module as well.
        fn leave(dna_hash: DnaHash, agent_pub_key: AgentPubKey) -> ();

        /// Invoke a zome function on a remote node (if you have been granted the capability).
        fn call_remote(
            dna_hash: DnaHash,
            from_agent: AgentPubKey,
            to_agent: AgentPubKey,
            zome_name: ZomeName,
            fn_name: String,
            cap: CapSecret,
            request: SerializedBytes,
        ) -> SerializedBytes;

        /// Publish data to the correct neigborhood.
        fn publish(
            dna_hash: DnaHash,
            from_agent: AgentPubKey,
            request_validation_receipt: bool,
            dht_hash: holochain_types::composite_hash::AnyDhtHash,
            ops: Vec<(holo_hash::DhtOpHash, holochain_types::dht_op::DhtOp)>,
            timeout_ms: Option<u64>,
        ) -> ();

        /// Request a validation package.
        fn get_validation_package(input: GetValidationPackage) -> (); // TODO - proper return type

        /// Get an entry from the DHT.
        fn get(
            dna_hash: DnaHash,
            from_agent: AgentPubKey,
            dht_hash: holochain_types::composite_hash::AnyDhtHash,
            options: GetOptions,
        ) -> Vec<SerializedBytes>;

        /// Get links from the DHT.
        fn get_links(input: GetLinks) -> (); // TODO - proper return type

        /// Send a validation receipt to a remote node.
        fn send_validation_receipt(dna_hash: DnaHash, agent_pub_key: AgentPubKey, receipt: SerializedBytes) -> ();
    }
}

impl HolochainP2pSender {
    /// Partially apply dna_hash && agent_pub_key to this sender,
    /// binding it to a specific cell context.
    pub fn into_cell(self, dna_hash: DnaHash, from_agent: AgentPubKey) -> crate::HolochainP2pCell {
        crate::HolochainP2pCell {
            sender: self,
            dna_hash: Arc::new(dna_hash),
            from_agent: Arc::new(from_agent),
        }
    }

    /// Clone and partially apply dna_hash && agent_pub_key to this sender,
    /// binding it to a specific cell context.
    pub fn to_cell(&self, dna_hash: DnaHash, from_agent: AgentPubKey) -> crate::HolochainP2pCell {
        self.clone().into_cell(dna_hash, from_agent)
    }
}