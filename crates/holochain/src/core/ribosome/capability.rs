use super::HostContext;
use super::WasmRibosome;
use holochain_zome_types::CapabilityInput;
use holochain_zome_types::CapabilityOutput;
use std::sync::Arc;

pub async fn capability(
    _ribosome: Arc<WasmRibosome>,
    _host_context: Arc<HostContext>,
    _input: CapabilityInput,
) -> CapabilityOutput {
    unimplemented!();
}