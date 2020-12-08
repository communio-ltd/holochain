//! # System Validation Checks
//! This module contains all the checks we run for sys validation

use super::queue_consumer::TriggerSender;
use super::state::metadata::ChainItemKey;
use super::state::metadata::MetadataBufT;
use super::workflow::incoming_dht_ops_workflow::incoming_dht_ops_workflow;
use super::workflow::sys_validation_workflow::SysValidationWorkspace;
use crate::conductor::api::CellConductorApiT;
use crate::conductor::entry_def_store::get_entry_def;
use fallible_iterator::FallibleIterator;
use holochain_keystore::AgentPubKeyExt;
use holochain_p2p::HolochainP2pCell;
use holochain_state::env::EnvironmentWrite;
use holochain_state::error::DatabaseResult;
use holochain_state::fresh_reader;
use holochain_types::dht_op::DhtOp;
use holochain_types::header::NewEntryHeaderRef;
use holochain_types::Entry;
use holochain_zome_types::element::ElementEntry;
use holochain_zome_types::entry_def::EntryDef;
use holochain_zome_types::entry_def::EntryVisibility;
use holochain_zome_types::header::AppEntryType;
use holochain_zome_types::header::EntryType;
use holochain_zome_types::header::Update;
use holochain_zome_types::link::LinkTag;
use holochain_zome_types::signature::Signature;
use holochain_zome_types::validate::ValidationStatus;
use holochain_zome_types::Header;
use std::convert::TryInto;

pub use crate::core::state::source_chain::SourceChainError;
pub use crate::core::state::source_chain::SourceChainResult;
pub(super) use error::*;

pub use holo_hash::*;
pub use holochain_types::element::Element;
pub use holochain_types::element::ElementExt;
pub use holochain_types::HeaderHashed;
pub use holochain_types::Timestamp;

#[allow(missing_docs)]
mod error;
#[cfg(test)]
mod tests;

/// 16mb limit on Entries due to websocket limits.
/// Consider splitting large entries up.
pub const MAX_ENTRY_SIZE: usize = 16_000_000;

/// 400b limit on LinkTags.
/// Tags are used as keys to the database to allow
/// fast lookup so they need to be small.
pub const MAX_TAG_SIZE: usize = 400;

/// Verify the signature for this header
pub async fn verify_header_signature(
    sig: &Signature,
    header: &Header,
) -> SysValidationResult<bool> {
    if header.author().verify_signature(sig, header).await? {
        Ok(true)
    } else {
        Ok(false)
    }
}

/// Verify the author key was valid at the time
/// of signing with dpki
/// TODO: This is just a stub until we have dpki.
pub async fn author_key_is_valid(_author: &AgentPubKey) -> SysValidationResult<bool> {
    Ok(true)
}

/// Check that previous header makes sense
/// for this header.
/// If not Dna then cannot be root of chain
/// and must have previous header
pub fn check_prev_header(header: &Header) -> SysValidationResult<()> {
    match &header {
        Header::Dna(_) => Ok(()),
        _ => {
            if header.header_seq() > 0 {
                header
                    .prev_header()
                    .ok_or(PrevHeaderError::MissingPrev)
                    .map_err(ValidationOutcome::from)?;
                Ok(())
            } else {
                Err(PrevHeaderError::InvalidRoot).map_err(|e| ValidationOutcome::from(e).into())
            }
        }
    }
}

/// Check that Dna headers are only added to empty source chains
pub async fn check_valid_if_dna(
    header: &Header,
    meta_vault: &impl MetadataBufT,
) -> SysValidationResult<()> {
    fresh_reader!(meta_vault.env(), |r| {
        match header {
            Header::Dna(_) => meta_vault
                .get_activity(&r, ChainItemKey::Agent(header.author().clone()))?
                .next()?
                .map_or(Ok(()), |_| {
                    Err(PrevHeaderError::InvalidRoot).map_err(|e| ValidationOutcome::from(e).into())
                }),
            _ => Ok(()),
        }
    })
}

// TODO: I think this can be removed now as rollbacks are detected when inserting
// metadata into the metadata buf.
/// Check if there are other headers at this
/// sequence number
pub async fn check_chain_rollback(
    header: &Header,
    workspace: &SysValidationWorkspace,
) -> SysValidationResult<()> {
    let header_hash = HeaderHash::with_data_sync(header);
    let k = ChainItemKey::AgentStatusSequence(
        header.author().clone(),
        ValidationStatus::Valid,
        header.header_seq(),
    );
    let env = workspace.meta_vault.env();
    // Check there are no conflicting chain items
    // at any valid or potentially valid stores.
    let count = fresh_reader!(env, |r| {
        let vault_count = workspace
            .meta_vault
            .get_activity(&r, k.clone())?
            .filter(|thh| Ok(thh.header_hash != header_hash))
            .count()?;
        let pending_count = workspace
            .meta_pending
            .get_activity(&r, k.clone())?
            .filter(|thh| Ok(thh.header_hash != header_hash))
            .count()?;
        DatabaseResult::Ok(vault_count + pending_count)
    })?;

    // Ok or log warning
    if count == 0 {
        return Ok(());
    } else {
        let s = tracing::warn_span!("agent_activity");
        let _g = s.enter();
        // TODO: implement real rollback detection once we know what that looks like
        tracing::error!(
            "Chain rollback detected at position {} for agent {:?} from header {:?}
            There were {} headers at this position",
            header.header_seq(),
            header.author(),
            header,
            count,
        );
    }
    Ok(())
}

/// Placeholder for future spam check.
/// Check header timestamps don't exceed MAX_PUBLISH_FREQUENCY
pub async fn check_spam(_header: &Header) -> SysValidationResult<()> {
    Ok(())
}

/// Check previous header timestamp is before this header
pub fn check_prev_timestamp(header: &Header, prev_header: &Header) -> SysValidationResult<()> {
    if header.timestamp() > prev_header.timestamp() {
        Ok(())
    } else {
        Err(PrevHeaderError::Timestamp).map_err(|e| ValidationOutcome::from(e).into())
    }
}

/// Check the previous header is one less then the current
pub fn check_prev_seq(header: &Header, prev_header: &Header) -> SysValidationResult<()> {
    let header_seq = header.header_seq();
    let prev_seq = prev_header.header_seq();
    if header_seq > 0 && prev_seq == header_seq - 1 {
        Ok(())
    } else {
        Err(PrevHeaderError::InvalidSeq(header_seq, prev_seq))
            .map_err(|e| ValidationOutcome::from(e).into())
    }
}

/// Check the entry variant matches the variant in the headers entry type
pub fn check_entry_type(entry_type: &EntryType, entry: &Entry) -> SysValidationResult<()> {
    match (entry_type, entry) {
        (EntryType::AgentPubKey, Entry::Agent(_)) => Ok(()),
        (EntryType::App(_), Entry::App(_)) => Ok(()),
        (EntryType::CapClaim, Entry::CapClaim(_)) => Ok(()),
        (EntryType::CapGrant, Entry::CapGrant(_)) => Ok(()),
        _ => Err(ValidationOutcome::EntryType.into()),
    }
}

/// Check the AppEntryType is valid for the zome.
/// Check the EntryDefId and ZomeId are in range.
pub async fn check_app_entry_type(
    entry_type: &AppEntryType,
    conductor_api: &impl CellConductorApiT,
) -> SysValidationResult<EntryDef> {
    let zome_index = u8::from(entry_type.zome_id()) as usize;
    // We want to be careful about holding locks open to the conductor api
    // so calls are made in blocks
    let dna_file = conductor_api.get_this_dna().await.map_err(Box::new)?;

    // Check if the zome is found
    let zome = dna_file
        .dna()
        .zomes
        .get(zome_index)
        .ok_or_else(|| ValidationOutcome::ZomeId(entry_type.clone()))?
        .clone()
        .1;

    let entry_def = get_entry_def(entry_type.id(), zome, dna_file.dna(), conductor_api).await?;

    // Check the visibility and return
    match entry_def {
        Some(entry_def) => {
            if entry_def.visibility == *entry_type.visibility() {
                Ok(entry_def)
            } else {
                Err(ValidationOutcome::EntryVisibility(entry_type.clone()).into())
            }
        }
        None => Err(ValidationOutcome::EntryDefId(entry_type.clone()).into()),
    }
}

/// Check the app entry type isn't private for store entry
pub fn check_not_private(entry_def: &EntryDef) -> SysValidationResult<()> {
    match entry_def.visibility {
        EntryVisibility::Public => Ok(()),
        EntryVisibility::Private => Err(ValidationOutcome::PrivateEntry.into()),
    }
}

/// Check the headers entry hash matches the hash of the entry
pub async fn check_entry_hash(hash: &EntryHash, entry: &Entry) -> SysValidationResult<()> {
    if *hash == EntryHash::with_data_sync(entry) {
        Ok(())
    } else {
        Err(ValidationOutcome::EntryHash.into())
    }
}

/// Check the header should have an entry.
/// Is either a Create or Update
pub fn check_new_entry_header(header: &Header) -> SysValidationResult<()> {
    match header {
        Header::Create(_) | Header::Update(_) => Ok(()),
        _ => Err(ValidationOutcome::NotNewEntry(header.clone()).into()),
    }
}

/// Check the entry size is under the MAX_ENTRY_SIZE
pub fn check_entry_size(entry: &Entry) -> SysValidationResult<()> {
    match entry {
        Entry::App(bytes) => {
            let size = std::mem::size_of_val(&bytes.bytes()[..]);
            if size < MAX_ENTRY_SIZE {
                Ok(())
            } else {
                Err(ValidationOutcome::EntryTooLarge(size, MAX_ENTRY_SIZE).into())
            }
        }
        // Other entry types are small
        _ => Ok(()),
    }
}

/// Check the link tag size is under the MAX_TAG_SIZE
pub fn check_tag_size(tag: &LinkTag) -> SysValidationResult<()> {
    let size = std::mem::size_of_val(&tag.0[..]);
    if size < MAX_TAG_SIZE {
        Ok(())
    } else {
        Err(ValidationOutcome::TagTooLarge(size, MAX_TAG_SIZE).into())
    }
}

/// Check a Update's entry type is the same for
/// original and new entry.
pub fn check_update_reference(
    eu: &Update,
    original_entry_header: &NewEntryHeaderRef<'_>,
) -> SysValidationResult<()> {
    if eu.entry_type == *original_entry_header.entry_type() {
        Ok(())
    } else {
        Err(ValidationOutcome::UpdateTypeMismatch(
            eu.entry_type.clone(),
            original_entry_header.entry_type().clone(),
        )
        .into())
    }
}

/// If we are not holding this header then
/// retrieve it and send it as a RegisterAddLink DhtOp
/// to our incoming_dht_ops_workflow.
///
/// Apply a checks callback to the Element.
///
/// Additionally sys validation will be triggered to
/// run again if we weren't holding it.
pub async fn check_and_hold_register_add_link<F>(
    hash: &HeaderHash,
    workspace: &mut SysValidationWorkspace,
    network: HolochainP2pCell,
    incoming_dht_ops_sender: Option<IncomingDhtOpSender>,
    f: F,
) -> SysValidationResult<()>
where
    F: FnOnce(&Element) -> SysValidationResult<()>,
{
    let source = check_and_hold(hash, workspace, network).await?;
    f(source.as_ref())?;
    if let (Some(incoming_dht_ops_sender), Source::Network(element)) =
        (incoming_dht_ops_sender, source)
    {
        incoming_dht_ops_sender
            .send_register_add_link(element)
            .await?;
    }
    Ok(())
}

/// If we are not holding this header then
/// retrieve it and send it as a RegisterAgentActivity DhtOp
/// to our incoming_dht_ops_workflow.
///
/// Apply a checks callback to the Element.
///
/// Additionally sys validation will be triggered to
/// run again if we weren't holding it.
pub async fn check_and_hold_register_agent_activity<F>(
    hash: &HeaderHash,
    workspace: &mut SysValidationWorkspace,
    network: HolochainP2pCell,
    incoming_dht_ops_sender: Option<IncomingDhtOpSender>,
    f: F,
) -> SysValidationResult<()>
where
    F: FnOnce(&Element) -> SysValidationResult<()>,
{
    let source = check_and_hold(hash, workspace, network).await?;
    f(source.as_ref())?;
    if let (Some(incoming_dht_ops_sender), Source::Network(element)) =
        (incoming_dht_ops_sender, source)
    {
        incoming_dht_ops_sender
            .send_register_agent_activity(element)
            .await?;
    }
    Ok(())
}

/// If we are not holding this header then
/// retrieve it and send it as a StoreEntry DhtOp
/// to our incoming_dht_ops_workflow.
///
/// Apply a checks callback to the Element.
///
/// Additionally sys validation will be triggered to
/// run again if we weren't holding it.
pub async fn check_and_hold_store_entry<F>(
    hash: &HeaderHash,
    workspace: &mut SysValidationWorkspace,
    network: HolochainP2pCell,
    incoming_dht_ops_sender: Option<IncomingDhtOpSender>,
    f: F,
) -> SysValidationResult<()>
where
    F: FnOnce(&Element) -> SysValidationResult<()>,
{
    let source = check_and_hold(hash, workspace, network).await?;
    f(source.as_ref())?;
    if let (Some(incoming_dht_ops_sender), Source::Network(element)) =
        (incoming_dht_ops_sender, source)
    {
        incoming_dht_ops_sender.send_store_entry(element).await?;
    }
    Ok(())
}

/// If we are not holding this entry then
/// retrieve any element at this EntryHash
/// and send it as a StoreEntry DhtOp
/// to our incoming_dht_ops_workflow.
///
/// Note this is different to check_and_hold_store_entry
/// because it gets the Element via an EntryHash which
/// means it will be any Element.
///
/// Apply a checks callback to the Element.
///
/// Additionally sys validation will be triggered to
/// run again if we weren't holding it.
pub async fn check_and_hold_any_store_entry<F>(
    hash: &EntryHash,
    workspace: &mut SysValidationWorkspace,
    network: HolochainP2pCell,
    incoming_dht_ops_sender: Option<IncomingDhtOpSender>,
    f: F,
) -> SysValidationResult<()>
where
    F: FnOnce(&Element) -> SysValidationResult<()>,
{
    let source = check_and_hold(hash, workspace, network).await?;
    f(source.as_ref())?;
    if let (Some(incoming_dht_ops_sender), Source::Network(element)) =
        (incoming_dht_ops_sender, source)
    {
        incoming_dht_ops_sender.send_store_entry(element).await?;
    }
    Ok(())
}

/// If we are not holding this header then
/// retrieve it and send it as a StoreElement DhtOp
/// to our incoming_dht_ops_workflow.
///
/// Apply a checks callback to the Element.
///
/// Additionally sys validation will be triggered to
/// run again if we weren't holding it.
pub async fn check_and_hold_store_element<F>(
    hash: &HeaderHash,
    workspace: &mut SysValidationWorkspace,
    network: HolochainP2pCell,
    incoming_dht_ops_sender: Option<IncomingDhtOpSender>,
    f: F,
) -> SysValidationResult<()>
where
    F: FnOnce(&Element) -> SysValidationResult<()>,
{
    let source = check_and_hold(hash, workspace, network).await?;
    f(source.as_ref())?;
    if let (Some(incoming_dht_ops_sender), Source::Network(element)) =
        (incoming_dht_ops_sender, source)
    {
        incoming_dht_ops_sender.send_store_element(element).await?;
    }
    Ok(())
}

/// Allows you to send an op to the
/// incoming_dht_ops_workflow if you
/// found it on the network and were supposed
/// to be holding it.
#[derive(derive_more::Constructor)]
pub struct IncomingDhtOpSender {
    env: EnvironmentWrite,
    sys_validation_trigger: TriggerSender,
}

impl IncomingDhtOpSender {
    /// Sends the op to the incoming workflow
    async fn send_op(
        self,
        element: Element,
        make_op: fn(Element) -> Option<(DhtOpHash, DhtOp)>,
    ) -> SysValidationResult<()> {
        if let Some(op) = make_op(element) {
            let ops = vec![op];
            incoming_dht_ops_workflow(&self.env, self.sys_validation_trigger, ops, None)
                .await
                .map_err(Box::new)?;
        }
        Ok(())
    }
    async fn send_store_element(self, element: Element) -> SysValidationResult<()> {
        self.send_op(element, make_store_element).await
    }
    async fn send_store_entry(self, element: Element) -> SysValidationResult<()> {
        self.send_op(element, make_store_entry).await
    }
    async fn send_register_add_link(self, element: Element) -> SysValidationResult<()> {
        self.send_op(element, make_register_add_link).await
    }
    async fn send_register_agent_activity(self, element: Element) -> SysValidationResult<()> {
        self.send_op(element, make_register_agent_activity).await
    }
}

/// Where the element was found.
enum Source {
    /// Locally because we are holding it or
    /// because we will be soon
    Local(Element),
    /// On the network.
    /// This means we aren't holding it so
    /// we should add it to our incoming ops
    Network(Element),
}

impl AsRef<Element> for Source {
    fn as_ref(&self) -> &Element {
        match self {
            Source::Local(el) | Source::Network(el) => el,
        }
    }
}

/// Check if we are holding a dependency and
/// run a check callback on the it.
/// This function also returns where the dependency
/// was found so you can decide whether or not to add
/// it to the incoming ops.
async fn check_and_hold<I: Into<AnyDhtHash> + Clone>(
    hash: &I,
    workspace: &mut SysValidationWorkspace,
    network: HolochainP2pCell,
) -> SysValidationResult<Source> {
    let hash: AnyDhtHash = hash.clone().into();
    // Create a workspace with just the local stores
    let mut local_cascade = workspace.local_cascade();
    if let Some(el) = local_cascade
        .retrieve(hash.clone(), Default::default())
        .await?
    {
        return Ok(Source::Local(el));
    }
    // Create a workspace with just the network
    let mut network_only_cascade = workspace.network_only_cascade(network);
    match network_only_cascade
        .retrieve(hash.clone(), Default::default())
        .await?
    {
        Some(el) => Ok(Source::Network(el)),
        None => Err(ValidationOutcome::NotHoldingDep(hash).into()),
    }
}

/// Make a StoreElement DhtOp from an Element.
/// Note that this can fail if the op is missing an
/// Entry when it was supposed to have one.
///
/// Because adding ops to incoming limbo while we are checking them
/// is only faster then waiting for them through gossip we don't care enough
/// to return an error.
fn make_store_element(element: Element) -> Option<(DhtOpHash, DhtOp)> {
    // Extract the data
    let (shh, element_entry) = element.into_inner();
    let (header, signature) = shh.into_header_and_signature();
    let header = header.into_content();

    // Check the entry
    let maybe_entry_box = match element_entry {
        ElementEntry::Present(e) => Some(e.into()),
        // This is ok because we weren't expecting an entry
        ElementEntry::NotApplicable | ElementEntry::Hidden => None,
        // The element is expected to have an entry but it wasn't
        // stored so we can't add this to incoming ops
        ElementEntry::NotStored => return None,
    };

    // Create the hash and op
    let op = DhtOp::StoreElement(signature, header, maybe_entry_box);
    let hash = DhtOpHash::with_data_sync(&op);
    Some((hash, op))
}

/// Make a StoreEntry DhtOp from an Element.
/// Note that this can fail if the op is missing an Entry or
/// the header is the wrong type.
///
/// Because adding ops to incoming limbo while we are checking them
/// is only faster then waiting for them through gossip we don't care enough
/// to return an error.
fn make_store_entry(element: Element) -> Option<(DhtOpHash, DhtOp)> {
    // Extract the data
    let (shh, element_entry) = element.into_inner();
    let (header, signature) = shh.into_header_and_signature();

    // Check the entry and exit early if it's not there
    let entry_box = element_entry.into_option()?.into();
    // If the header is the wrong type exit early
    let header = header.into_content().try_into().ok()?;

    // Create the hash and op
    let op = DhtOp::StoreEntry(signature, header, entry_box);
    let hash = DhtOpHash::with_data_sync(&op);
    Some((hash, op))
}

/// Make a RegisterAddLink DhtOp from an Element.
/// Note that this can fail if the header is the wrong type
///
/// Because adding ops to incoming limbo while we are checking them
/// is only faster then waiting for them through gossip we don't care enough
/// to return an error.
fn make_register_add_link(element: Element) -> Option<(DhtOpHash, DhtOp)> {
    // Extract the data
    let (shh, _) = element.into_inner();
    let (header, signature) = shh.into_header_and_signature();

    // If the header is the wrong type exit early
    let header = header.into_content().try_into().ok()?;

    // Create the hash and op
    let op = DhtOp::RegisterAddLink(signature, header);
    let hash = DhtOpHash::with_data_sync(&op);
    Some((hash, op))
}

/// Make a RegisterAgentActivity DhtOp from an Element.
/// Note that this can fail if the header is the wrong type
///
/// Because adding ops to incoming limbo while we are checking them
/// is only faster then waiting for them through gossip we don't care enough
/// to return an error.
fn make_register_agent_activity(element: Element) -> Option<(DhtOpHash, DhtOp)> {
    // Extract the data
    let (shh, _) = element.into_inner();
    let (header, signature) = shh.into_header_and_signature();

    // If the header is the wrong type exit early
    let header = header.into_content();

    // Create the hash and op
    let op = DhtOp::RegisterAgentActivity(signature, header);
    let hash = DhtOpHash::with_data_sync(&op);
    Some((hash, op))
}