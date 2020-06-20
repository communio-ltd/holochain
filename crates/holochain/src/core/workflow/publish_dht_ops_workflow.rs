//! The workflow and queue consumer for sys validation

use super::*;
use crate::core::{
    queue_consumer::{OneshotWriter, WorkComplete},
    state::workspace::{Workspace, WorkspaceResult},
};
use error::WorkflowResult;
use holochain_state::prelude::{GetDb, Reader, Writer};
use tracing::*;

pub async fn publish_dht_ops_workflow<'env>(
    workspace: PublishDhtOpsWorkspace<'env>,
    writer: OneshotWriter,
) -> WorkflowResult<WorkComplete> {
    warn!("unimplemented");

    // --- END OF WORKFLOW, BEGIN FINISHER BOILERPLATE ---

    // commit the workspace
    writer
        .with_writer(|writer| workspace.flush_to_txn(writer).expect("TODO"))
        .await?;

    // trigger other workflows
    // (n/a)

    Ok(WorkComplete::Complete)
}

pub struct PublishDhtOpsWorkspace<'env>(std::marker::PhantomData<&'env ()>);

impl<'env> PublishDhtOpsWorkspace<'env> {}

impl<'env> Workspace<'env> for PublishDhtOpsWorkspace<'env> {
    /// Constructor
    #[allow(dead_code)]
    fn new(reader: &'env Reader<'env>, dbs: &impl GetDb) -> WorkspaceResult<Self> {
        Ok(Self(std::marker::PhantomData))
    }
    fn flush_to_txn(self, writer: &mut Writer) -> WorkspaceResult<()> {
        warn!("unimplemented");
        Ok(())
    }
}
