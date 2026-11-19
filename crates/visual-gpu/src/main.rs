//! `visual-gpu`: the GPU's time benchmark (see the library's `bench` module).

#![forbid(unsafe_code)]
#![expect(
    clippy::multiple_crate_versions,
    reason = "wgpu's own tree holds two versions of hashbrown (naga's indexmap, gpu-allocator) \
              and of syn (bytemuck's and zerocopy's derives), which this crate cannot choose; \
              each pair is a reviewed exception of deny.toml"
)]

use std::process::ExitCode;

/// Runs the requested command on the first GPU and the baseline below the current
/// directory, the repository root.
///
/// Exit codes: 0 on success, 1 when the command failed, 2 for a usage error; every error
/// is one `error:` line on stderr. `visual-gpu help` prints the usage on stdout. A closed
/// stdout is not an error.
fn main() -> ExitCode {
    let args: Vec<String> = std::env::args_os()
        .skip(1)
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect();
    let mut system = visual_gpu::bench::System::new(std::path::PathBuf::from("."));
    let Err(error) = visual_gpu::bench::run(&args, &mut system, &mut std::io::stdout()) else {
        return ExitCode::SUCCESS;
    };
    // Nothing more can be done if stderr itself is broken: the exit code still tells.
    drop(std::io::Write::write_all(
        &mut std::io::stderr(),
        format!("error: {}\n", error.line).as_bytes(),
    ));
    ExitCode::from(error.code)
}
