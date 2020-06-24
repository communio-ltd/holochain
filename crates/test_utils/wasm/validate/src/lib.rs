use holochain_wasmer_guest::*;
use holochain_zome_types::*;
use holochain_zome_types::validate::ValidateCallbackResult;

holochain_wasmer_guest::holochain_externs!();

/// an example inner value that can be serialized into the contents of Entry::App()
#[derive(Deserialize, Serialize, SerializedBytes)]
enum ThisWasmEntry {
    AlwaysValidates,
    NeverValidates,
}

#[no_mangle]
pub extern "C" fn validate(host_allocation_ptr: GuestPtr) -> GuestPtr {
    // load host args
    let input: HostInput = host_args!(host_allocation_ptr);

    // extract the entry to validate
    let result: ValidateCallbackResult = match Entry::try_from(input.into_inner()) {
        // we do want to validate our app entries
        Ok(Entry::App(serialized_bytes)) => match ThisWasmEntry::try_from(serialized_bytes) {
            // the AlwaysValidates variant passes
            Ok(ThisWasmEntry::AlwaysValidates) => ValidateCallbackResult::Valid,
            // the NeverValidates variants fails
            Ok(ThisWasmEntry::NeverValidates) => ValidateCallbackResult::Invalid("NeverValidates never validates".to_string()),
            _ => ValidateCallbackResult::Invalid("Couldn't get ThisWasmEntry from the app entry".to_string()),
        },
        // other entry types we don't care about
        Ok(_) => ValidateCallbackResult::Valid,
        _ => ValidateCallbackResult::Invalid("Couldn't get App serialized bytes from host input".to_string()),
    };

    ret!(GuestOutput::new(try_result!(result.try_into(), "failed to serialize return value".to_string())));
}

/// we can write normal rust code with Results outside our externs
fn _commit_validate(to_commit: ThisWasmEntry) -> Result<GuestOutput, String> {
    let commit_output: CommitEntryOutput = host_call!(__commit_entry, CommitEntryInput::new(Entry::App(to_commit.try_into()?)))?;
    Ok(GuestOutput::new(commit_output.try_into()?))
}

#[no_mangle]
pub extern "C" fn always_validates(_: GuestPtr) -> GuestPtr {
    ret!(try_result!(_commit_validate(ThisWasmEntry::AlwaysValidates), "error processing commit"))
}
#[no_mangle]
pub extern "C" fn never_validates(_: GuestPtr) -> GuestPtr {
    ret!(try_result!(_commit_validate(ThisWasmEntry::NeverValidates), "error processing commit"))
}