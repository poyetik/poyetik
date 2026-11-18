//! Tests of the pinned tools and the environment the check refuses, checked before any step.

use std::ffi::OsString;
use std::path::Path;

use crate::tests::{
    CHANGES_MODE, QUALITY, TOOLCHAIN, failed, found, passing, scratch, system, tool_answer,
};
use crate::{
    CommandFailure, Exit, Failed, Failure, Runner, check_steps, preflight_gate, run_steps,
};

/// An environment that lets the checks run: no build variable, no Cargo configuration from
/// outside the repository.
const CLEAN: crate::Environment = crate::Environment {
    variables: Vec::new(),
    cargo_configs: Vec::new(),
};

#[test]
fn checks_the_tools_and_the_platform_first() {
    let present = |node: &str| -> Vec<Result<String, CommandFailure>> {
        crate::TOOLS
            .iter()
            .map(|tool| {
                Ok(if tool.program == "node" {
                    node.to_owned()
                } else {
                    tool_answer(tool.program, tool.args.first().copied())
                })
            })
            .collect()
    };
    assert_eq!(
        preflight_gate(
            "macos",
            &present("v22.23.4\n"),
            "22.23.4\n",
            TOOLCHAIN,
            &CLEAN
        ),
        Ok(()),
        "the release of .node-version"
    );
    for (node, pinned) in [
        ("v22.24.0", "22.23.4"),
        ("v22.23.3", "22.23.4"),
        ("v24.1.0", "22.23.4"),
        ("garbage", "22.23.4"),
        ("v22.23.4", "22.23"),
    ] {
        assert_eq!(
            preflight_gate("linux", &present(node), pinned, TOOLCHAIN, &CLEAN),
            found(
                "the checks cannot run here",
                &[format!(
                    "Node.js {node} is installed, the checks need {pinned}: install the Node.js \
                     release of .node-version (fnm, nvm and volta read it)"
                )]
            ),
            "{node} against {pinned}"
        );
    }
    let mut missing = present("v22.23.4");
    missing[5] = Err(CommandFailure {
        line: "cargo mutants --version".to_owned(),
        failed: Failed::Ended {
            exit: Exit::Code(101_i32),
            reason: Some("error: no such command: `mutants`".to_owned()),
        },
    });
    assert_eq!(
        preflight_gate("windows", &missing, "22.23.4", TOOLCHAIN, &CLEAN),
        found("the checks cannot run here", &[
            "the checks run on macOS and Linux: on Windows, run them in WSL2".to_owned(),
            "`cargo mutants --version` exited with code 101: error: no such command: `mutants`; to install: cargo install --locked cargo-mutants@27.1.0".to_owned(),
        ]),
        "platform, and a missing tool with its own reason and how to install it"
    );
    assert_eq!(
        crate::NODE_FLOOR,
        (22, 18),
        "the engines of every package.json"
    );
}

#[test]
fn requires_prlimit_on_linux_only() {
    let mut answers: Vec<Result<String, CommandFailure>> = crate::TOOLS
        .iter()
        .map(|tool| Ok(tool_answer(tool.program, tool.args.first().copied())))
        .collect();
    let at = crate::TOOLS
        .iter()
        .position(|tool| tool.program == "prlimit")
        .expect("prlimit among the tools");
    assert_eq!(crate::TOOLS[at].args, ["--version"], "asked its version");
    for os in ["linux", "macos"] {
        assert_eq!(
            preflight_gate(os, &answers, "22.23.4", TOOLCHAIN, &CLEAN),
            Ok(()),
            "{os}: installed"
        );
    }
    answers[at] = Err(CommandFailure {
        line: "prlimit --version".to_owned(),
        failed: Failed::NotStarted("No such file or directory (os error 2)".to_owned()),
    });
    assert_eq!(
        preflight_gate("macos", &answers, "22.23.4", TOOLCHAIN, &CLEAN),
        Ok(()),
        "not read off Linux, where the mutation tests run without it"
    );
    assert_eq!(
        preflight_gate("linux", &answers, "22.23.4", TOOLCHAIN, &CLEAN),
        failed(
            "the checks cannot run here: cannot run `prlimit --version`: No such file or \
             directory (os error 2); to install: install util-linux (its prlimit bounds the \
             memory of each test program of the mutation tests)"
        ),
        "missing on Linux, with how to install it"
    );
}

#[test]
fn requires_a_vulkan_driver_on_linux_only() {
    let mut answers: Vec<Result<String, CommandFailure>> = crate::TOOLS
        .iter()
        .map(|tool| Ok(tool_answer(tool.program, tool.args.first().copied())))
        .collect();
    let at = crate::TOOLS
        .iter()
        .position(|tool| tool.args.iter().any(|arg| arg.contains("vulkan/icd.d")))
        .expect("the Vulkan driver among the tools");
    assert_eq!(
        crate::TOOLS[at].args,
        [
            "-c",
            "ls /usr/share/vulkan/icd.d/*.json || ls /etc/vulkan/icd.d/*.json"
        ],
        "the two directories where a system declares its Vulkan drivers to the loader"
    );
    answers[at] = Err(CommandFailure {
        line: "sh -c ls".to_owned(),
        failed: Failed::Ended {
            exit: Exit::Code(2),
            reason: None,
        },
    });
    assert_eq!(
        preflight_gate("macos", &answers, "22.23.4", TOOLCHAIN, &CLEAN),
        Ok(()),
        "not read off Linux, where Metal needs no driver to install"
    );
    assert_eq!(
        preflight_gate("linux", &answers, "22.23.4", TOOLCHAIN, &CLEAN),
        failed(
            "the checks cannot run here: `sh -c ls` exited with code 2; to install: install a \
             Vulkan driver, which the GPU workspace's tests render on (on Ubuntu, \
             mesa-vulkan-drivers: lavapipe renders on the processor)"
        ),
        "missing on Linux, with how to install it"
    );
}

#[test]
fn requires_the_rust_release_of_the_toolchain_file() {
    let answers = |rustc: &str| -> Vec<Result<String, CommandFailure>> {
        crate::TOOLS
            .iter()
            .map(|tool| {
                Ok(if tool.program == "rustc" {
                    rustc.to_owned()
                } else {
                    tool_answer(tool.program, tool.args.first().copied())
                })
            })
            .collect()
    };
    assert_eq!(
        preflight_gate(
            "linux",
            &answers("rustc 1.99.1 (58cf8f9c3 2026-10-13)\n"),
            "22.23.4",
            TOOLCHAIN,
            &CLEAN
        ),
        Ok(()),
        "the channel of rust-toolchain.toml"
    );
    let hint = "the release of rust-toolchain.toml, which rustup installs: \
                remove what overrides it (`rustup override unset`)";
    for (rustc, toolchain) in [
        ("rustc 1.98.0 (aaaaaaaaa 2026-09-01)", TOOLCHAIN),
        ("rustc 1.99.1-beta.2 (bbbbbbbbb 2026-10-01)", TOOLCHAIN),
        ("rustc 1.100.0-nightly (ccccccccc 2026-11-01)", TOOLCHAIN),
        (
            "rustc 1.99.1 (58cf8f9c3 2026-10-13)",
            "[toolchain]\nchannel = \"1.99.10\"\n",
        ),
        ("rustc", TOOLCHAIN),
    ] {
        let channel = crate::toolchain_channel(toolchain).unwrap_or_default();
        assert_eq!(
            preflight_gate("linux", &answers(rustc), "22.23.4", toolchain, &CLEAN),
            found(
                "the checks cannot run here",
                &[format!(
                    "{rustc} is installed, the checks need {channel} of rust-toolchain.toml: {hint}"
                )]
            ),
            "{rustc} against {toolchain:?}"
        );
    }
    for toolchain in [
        "[toolchain]\nprofile = \"minimal\"\n",
        "[toolchain]\nchannel = 1.99.1\n",
        "[toolchain]\nchannels = \"1.99.1\"\n",
        "[toolchain]\nchannel = \"\"\n",
    ] {
        assert_eq!(crate::toolchain_channel(toolchain), None, "{toolchain:?}");
        assert_eq!(
            preflight_gate(
                "linux",
                &answers("rustc 1.99.1 (58cf8f9c3 2026-10-13)"),
                "22.23.4",
                toolchain,
                &CLEAN
            ),
            found(
                "the checks cannot run here",
                &["rust-toolchain.toml must pin one release: channel = \"X.Y.Z\"".to_owned()]
            ),
            "{toolchain:?}"
        );
    }
    assert_eq!(crate::toolchain_channel(TOOLCHAIN), Some("1.99.1"), "read");
    assert_eq!(
        crate::toolchain_channel("[toolchain]\n  channel   =   \"1.99.1\"  \n"),
        Some("1.99.1"),
        "spaces around"
    );
    let without = crate::runner::command("rustc", Path::new("."), &[]);
    assert!(
        without
            .get_envs()
            .any(|(name, value)| name == "RUSTUP_TOOLCHAIN" && value.is_none()),
        "no command inherits the toolchain rustup chose for xtask itself"
    );
}

#[test]
fn refuses_what_changes_cargo_from_outside_the_repository() {
    let answers: Vec<Result<String, CommandFailure>> = crate::TOOLS
        .iter()
        .map(|tool| Ok(tool_answer(tool.program, tool.args.first().copied())))
        .collect();
    let refused = [
        "RUSTFLAGS",
        "RUSTDOCFLAGS",
        "CARGO_ENCODED_RUSTFLAGS",
        "CARGO_ENCODED_RUSTDOCFLAGS",
        "CARGO_BUILD_TARGET",
        "CARGO_BUILD_RUSTFLAGS",
        "CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUSTFLAGS",
        "CARGO_PROFILE_RELEASE_OPT_LEVEL",
        "CARGO_UNSTABLE_BUILD_STD",
        // Nextest's own settings: a retried test, another profile, no test accepted.
        "NEXTEST_RETRIES",
        "NEXTEST_PROFILE",
        "NEXTEST_NO_TESTS",
        "RUSTC",
        "RUSTC_WRAPPER",
        "RUSTC_WORKSPACE_WRAPPER",
        "RUSTDOC",
        "CLIPPY_CONF_DIR",
        // The compiler acting as a nightly one, which only the doc tests of the codes set.
        "RUSTC_BOOTSTRAP",
        // The engine block a build names: the checks build every engine block.
        "POYETIK_ENGINE_BLOCK",
    ];
    let accepted = [
        "CARGO_TARGET_DIR",
        "CARGO_HOME",
        "CARGO_MANIFEST_DIR",
        "RUSTUP_TOOLCHAIN",
        "RUSTUP_HOME",
        "RUSTC_BOOTSTRAP_X",
        "XRUSTFLAGS",
        "rustflags",
        "CARGO_TARGET",
        "POYETIK_ENGINE_BLOCKS",
        "NEXTESTX",
        "PATH",
    ];
    for name in refused {
        assert!(crate::refused_variable(name), "{name}");
    }
    for name in accepted {
        assert!(!crate::refused_variable(name), "{name}");
    }
    let hostile = crate::Environment {
        variables: refused
            .iter()
            .chain(&accepted)
            .map(|name| (*name).to_owned())
            .collect(),
        cargo_configs: vec!["/home/x/.cargo/config.toml".to_owned()],
    };
    let mut problems: Vec<String> = refused
        .iter()
        .map(|name| {
            format!("{name} is set, which changes what cargo builds or how it lints: unset it")
        })
        .collect();
    problems.push(
        "/home/x/.cargo/config.toml configures every cargo command here: move it away (the \
         configuration of the repository is .cargo/config.toml)"
            .to_owned(),
    );
    assert_eq!(
        preflight_gate("linux", &answers, "22.23.4", TOOLCHAIN, &hostile),
        found("the checks cannot run here", &problems),
        "each by name"
    );
    let mut fake = passing();
    fake.environment.variables = vec!["RUSTFLAGS".to_owned()];
    assert_eq!(
        run_steps(&check_steps(CHANGES_MODE), &mut fake, &mut Vec::new()),
        failed(
            "Tools and platform: the checks cannot run here: RUSTFLAGS is set, which changes \
             what cargo builds or how it lints: unset it"
        ),
        "the check reads the environment of its runner"
    );
}

#[test]
fn lists_the_cargo_configurations_outside_the_repository() {
    let root = Path::new("/a/b/repo");
    let configs = |cargo_home: Option<&str>, home: Option<&str>| -> Vec<String> {
        crate::cargo_config_candidates(
            root,
            cargo_home.map(OsString::from),
            home.map(OsString::from),
        )
        .iter()
        .map(|path| path.display().to_string())
        .collect()
    };
    let ancestors = [
        "/.cargo/config",
        "/.cargo/config.toml",
        "/a/.cargo/config",
        "/a/.cargo/config.toml",
        "/a/b/.cargo/config",
        "/a/b/.cargo/config.toml",
        "/a/b/repo/.cargo/config",
    ];
    let with = |home: &[&str]| -> Vec<String> {
        let mut all: Vec<String> = home
            .iter()
            .chain(&ancestors)
            .map(|path| (*path).to_owned())
            .collect();
        all.sort();
        all
    };
    assert_eq!(
        configs(Some("/c"), Some("/h")),
        with(&["/c/config", "/c/config.toml"]),
        "CARGO_HOME, and every directory from the root up, the repository's own file apart"
    );
    assert_eq!(
        configs(None, Some("/h")),
        with(&["/h/.cargo/config", "/h/.cargo/config.toml"]),
        "else the .cargo of the home directory"
    );
    assert_eq!(configs(None, None), with(&[]), "no home at all");
    assert_eq!(
        configs(Some("/a/.cargo"), None),
        with(&[]),
        "each file once"
    );

    let dir = scratch("cargo-configs");
    let repo = dir.join("repo");
    std::fs::create_dir_all(repo.join(".cargo")).expect("repository");
    std::fs::create_dir_all(dir.join(".cargo")).expect("above");
    for path in [
        dir.join(".cargo/config.toml"),
        repo.join(".cargo/config"),
        repo.join(".cargo/config.toml"),
    ] {
        std::fs::write(path, "").expect("configuration");
    }
    std::fs::create_dir_all(repo.join("x")).expect("a folder to walk out of");
    let environment = system(repo.join("x/..")).environment();
    let missing = system(dir.join("missing")).environment();
    let canonical = std::fs::canonicalize(&dir).expect("canonical");
    let listed = |path: &str| {
        environment
            .cargo_configs
            .contains(&canonical.join(path).display().to_string())
    };
    assert!(listed(".cargo/config.toml"), "above: {environment:?}");
    assert!(
        listed("repo/.cargo/config"),
        "the other name: {environment:?}"
    );
    assert!(
        !listed("repo/.cargo/config.toml"),
        "its own: {environment:?}"
    );
    assert!(
        environment
            .variables
            .contains(&"CARGO_MANIFEST_DIR".to_owned()),
        "the names of the variables: {environment:?}"
    );
    assert!(
        missing
            .cargo_configs
            .contains(&dir.join(".cargo/config.toml").display().to_string()),
        "a root that cannot be resolved is walked up as it is: {missing:?}"
    );
}

#[test]
fn refuses_cargo_tools_at_other_versions() {
    let answers: Vec<Result<String, CommandFailure>> = crate::TOOLS
        .iter()
        .map(|tool| Ok(tool_answer(tool.program, tool.args.first().copied())))
        .collect();
    assert_eq!(
        preflight_gate("linux", &answers, "22.23.4", TOOLCHAIN, &CLEAN),
        Ok(()),
        "the pinned versions"
    );
    let mut older = answers;
    older[5] = Ok("cargo-mutants 26.0.0\n".to_owned());
    older[6] = Ok("cargo-deny".to_owned());
    assert_eq!(
        preflight_gate("linux", &older, "22.23.4", TOOLCHAIN, &CLEAN),
        found("the checks cannot run here", &[
            "cargo-mutants 26.0.0 is installed, the checks need cargo-mutants 27.1.0: \
             cargo install --locked cargo-mutants@27.1.0".to_owned(),
            "cargo-deny is installed, the checks need cargo-deny 0.20.2: cargo install --locked cargo-deny@0.20.2".to_owned(),
        ]),
        "another version, no version"
    );
}

#[test]
fn renders_install_hints_from_the_pins() {
    use crate::{Install, NIGHTLY_TOOLCHAIN, Need, PINNED_TOOLS, TOOLS};
    for pin in PINNED_TOOLS {
        let tool = TOOLS
            .iter()
            .find(|tool| tool.need == Need::Pinned(pin))
            .expect("every pinned tool is checked");
        assert_eq!(
            tool.install.to_string(),
            format!("cargo install --locked {}@{}", pin.0, pin.1),
            "{}: installed at the version the checks need",
            pin.0
        );
    }
    let nightly = NIGHTLY_TOOLCHAIN.trim_start_matches('+');
    for tool in TOOLS
        .iter()
        .filter(|tool| tool.args.contains(&NIGHTLY_TOOLCHAIN))
    {
        assert_eq!(tool.need, Need::Present, "any build of the pinned nightly");
        assert!(
            tool.install.to_string().contains(nightly),
            "{:?}: installs the pinned nightly",
            tool.args
        );
    }
    assert_eq!(
        Install::Nightly(&["a", "b"]).to_string(),
        format!("rustup toolchain install {nightly} --component a --component b"),
        "a toolchain with its components"
    );
    assert_eq!(
        Install::NightlyComponents(&["a", "b"]).to_string(),
        format!("rustup component add --toolchain {nightly} a b"),
        "components of the toolchain"
    );
    assert_eq!(
        Install::Node.to_string(),
        "install the Node.js release of .node-version (fnm, nvm and volta read it)",
        "Node.js from its pin"
    );
    assert_eq!(
        Install::Text("install git").to_string(),
        "install git",
        "what to do, in words"
    );
}

#[test]
fn reads_node_releases_alike_everywhere() {
    for (text, release) in [
        ("v22.18.0\n", Some((22, 18, 0))),
        ("22.23.4", Some((22, 23, 4))),
        ("v24.1.0", Some((24, 1, 0))),
        ("v22.18.0-nightly", None),
        ("v22.x", None),
        ("22.18", None),
        ("vv22.18.0", None),
        ("22.23.4.1", None),
    ] {
        assert_eq!(
            crate::node_release(text).map(|version| (version.major, version.minor, version.patch)),
            release,
            "{text:?}"
        );
        let answers: Vec<Result<String, CommandFailure>> = crate::TOOLS
            .iter()
            .map(|tool| {
                Ok(if tool.program == "node" {
                    text.to_owned()
                } else {
                    tool_answer(tool.program, tool.args.first().copied())
                })
            })
            .collect();
        assert_eq!(
            preflight_gate("linux", &answers, text, TOOLCHAIN, &CLEAN).is_ok(),
            crate::workflow_gate(&[crate::WORKFLOW.to_owned()], QUALITY, text).is_ok(),
            "{text:?}: the same verdict from the preflight and the workflow gate"
        );
    }
}

#[test]
fn requires_the_exact_cargo_configuration() {
    assert_eq!(
        crate::cargo_config_gate(crate::CARGO_CONFIG),
        Ok(()),
        "the alias"
    );
    for config in [
        crate::CARGO_CONFIG.replace(" --locked", ""),
        format!(
            "{}[build]\nrustflags = [\"-A{}_{}\"]\n",
            crate::CARGO_CONFIG,
            "unsafe",
            "code"
        ),
    ] {
        assert_eq!(
            crate::cargo_config_gate(&config),
            Err(Failure {
                summary: ".cargo/config.toml differs from the configuration xtask expects (no \
                          build flag, a locked alias)"
                    .to_owned(),
                details: vec![
                    "expected:".to_owned(),
                    crate::CARGO_CONFIG.to_owned(),
                    "found:".to_owned(),
                    config.clone(),
                ],
                usage: false,
            }),
            "{config}"
        );
    }
}

#[test]
fn requires_the_gpu_workspace_to_share_the_roots_settings() {
    let root = "[workspace]\n\n[workspace.package]\nversion = \"1.0.0\"\n\n[workspace.dependencies]\nx = 1\n\n[workspace.lints.rust]\nwarnings = \"deny\"\n\n[workspace.lints.rustdoc]\nbare_urls = \"deny\"\n\n[workspace.lints.clippy]\nall = \"deny\"\n\n[profile.release]\nlto = true\n";
    let gpu = "# Why.\n[workspace]\nresolver = \"3\"\nmembers = [\".\"]\n\n[workspace.package]\nversion = \"1.0.0\"\n\n[workspace.lints.rust]\nwarnings = \"deny\"\n\n[workspace.lints.rustdoc]\nbare_urls = \"deny\"\n\n[workspace.lints.clippy]\nall = \"deny\"\n\n[package]\nname = \"g\"\n\n[dependencies]\nwgpu = \"1\"\n\n[dev-dependencies]\n\n[lints]\nworkspace = true\n";
    let refused = |problems: &[&str]| {
        crate::steps::verdict(
            "the GPU workspace shares the root's package fields and lints",
            problems.iter().map(|&problem| problem.to_owned()).collect(),
        )
    };
    assert_eq!(
        crate::gpu_manifest_gate(root, gpu),
        Ok(()),
        "both tables held, and nothing else but its package and dependencies"
    );
    assert_eq!(
        crate::gpu_manifest_gate(
            root,
            &gpu.replace("wgpu = \"1\"", "wgpu = \"2\"\nbytes = \"1\"")
        ),
        Ok(()),
        "its own dependencies, which Dependabot updates"
    );
    let package =
        "crates/visual-gpu/Cargo.toml does not hold the root's [workspace.package] table exactly";
    let lints = "crates/visual-gpu/Cargo.toml does not hold the root's [workspace.lints.rust] table exactly";
    let sections = "crates/visual-gpu/Cargo.toml holds other sections than [workspace], \
                    [workspace.package], [workspace.lints.rust], [workspace.lints.rustdoc], \
                    [workspace.lints.clippy], [package], [dependencies], [dev-dependencies], \
                    [lints], in that order";
    let workspace = "crates/visual-gpu/Cargo.toml: its [workspace] table is not exactly \
                     `resolver = \"3\"` and `members = [\".\"]`";
    let own_lints =
        "crates/visual-gpu/Cargo.toml: its [lints] table is not exactly `workspace = true`";
    for (edited, problems, what) in [
        (
            gpu.replace("1.0.0", "1.0.1"),
            vec![package],
            "another version",
        ),
        (gpu.replace("deny", "warn"), vec![lints], "another lint"),
        (
            gpu.replace(
                "all = \"deny\"\n\n[package]",
                "all = \"deny\"\n\nfloat_cmp = \"allow\"\n[package]",
            ),
            vec![lints],
            "a lint after the root's table, before the package",
        ),
        (
            format!("{gpu}\n[profile.test]\nopt-level = 3\n"),
            // Its body read as the crate's lints' too.
            vec![sections, own_lints],
            "a profile",
        ),
        (
            format!("{gpu}\n[patch.crates-io]\nwgpu = {{ path = \"../w\" }}\n"),
            // Its body read as the crate's lints' too.
            vec![sections, own_lints],
            "a patch",
        ),
        (
            format!("{gpu}\n  [profile.dev]\ndebug = false\n"),
            // Its body read as the crate's lints' too.
            vec![sections, own_lints],
            "an indented header",
        ),
        (
            gpu.replace("members = [\".\"]", "members = [\".\", \"x\"]"),
            vec![workspace],
            "another member",
        ),
        (
            gpu.replace(
                "workspace = true\n",
                "workspace = true\nrust.unsafe_code = \"allow\"\n",
            ),
            vec![own_lints],
            "a lint of its own",
        ),
    ] {
        assert_eq!(
            crate::gpu_manifest_gate(root, &edited),
            refused(&problems),
            "{what}"
        );
    }
    assert_eq!(
        crate::gpu_manifest_gate(&root.replace("[profile.release]", "[profile.dev]"), gpu),
        refused(&["Cargo.toml has no [workspace.lints.rust] table followed by [profile.release]"]),
        "a root without the table's end"
    );
    assert_eq!(
        crate::gpu_manifest_gate("", gpu),
        refused(&[
            "Cargo.toml has no [workspace.package] table followed by [workspace.dependencies]",
            "Cargo.toml has no [workspace.lints.rust] table followed by [profile.release]",
        ]),
        "a root without either table"
    );
    assert_eq!(crate::GPU_MANIFEST, "crates/visual-gpu/Cargo.toml");
}

#[test]
fn requires_every_workspace_apart_to_share_the_roots_settings() {
    let root = "[workspace]\n\n[workspace.package]\nversion = \"1.0.0\"\n\n[workspace.dependencies]\nx = 1\n\n[workspace.lints.rust]\nwarnings = \"deny\"\n\n[profile.release]\nlto = true\n";
    let gpu = "[workspace]\nmembers = [\".\"]\n\n[workspace.package]\nversion = \"1.0.0\"\n\n[workspace.lints.rust]\nwarnings = \"deny\"\n\n[package]\nname = \"g\"\n";
    let wgsl = "[workspace]\nmembers = [\"t\"]\n\n[workspace.package]\nversion = \"1.0.0\"\n\n[workspace.lints.rust]\nwarnings = \"deny\"\n\n";
    let refused = |problems: &[&str]| {
        crate::steps::verdict(
            "every workspace apart shares the root's package fields and lints",
            problems.iter().map(|&problem| problem.to_owned()).collect(),
        )
    };
    let gate = |root_text: &str, gpu_text: &str, wgsl_text: &str| {
        crate::apart_manifests_gate(
            root_text,
            &[("g/Cargo.toml", gpu_text), ("w/Cargo.toml", wgsl_text)],
        )
    };
    assert_eq!(gate(root, gpu, wgsl), Ok(()), "both tables held by both");
    assert_eq!(
        gate(root, &gpu.replace("1.0.0", "1.0.1"), wgsl),
        refused(&["g/Cargo.toml does not hold the root's [workspace.package] table exactly"]),
        "another version in the first, named"
    );
    assert_eq!(
        gate(root, gpu, &wgsl.replace("deny", "warn")),
        refused(&["w/Cargo.toml does not hold the root's [workspace.lints.rust] table exactly"]),
        "another lint in the second, named"
    );
    assert_eq!(
        gate(
            root,
            &gpu.replace("deny", "warn"),
            &wgsl.replace("deny", "warn")
        ),
        refused(&[
            "g/Cargo.toml does not hold the root's [workspace.lints.rust] table exactly",
            "w/Cargo.toml does not hold the root's [workspace.lints.rust] table exactly",
        ]),
        "each manifest said"
    );
    assert_eq!(
        gate(
            root,
            &gpu.replace("\n\n[package]", "\nunsafe_code = \"allow\"\n\n[package]"),
            &format!("{wgsl}unsafe_code = \"allow\"\n")
        ),
        refused(&[
            "g/Cargo.toml does not hold the root's [workspace.lints.rust] table exactly",
            "w/Cargo.toml does not hold the root's [workspace.lints.rust] table exactly",
        ]),
        "a key added after the table, before the next or at the end, extends it"
    );
    assert_eq!(
        gate(
            root,
            &gpu.replace(
                "\n\n[workspace.lints",
                "\n\npublish = true\n[workspace.lints"
            ),
            wgsl
        ),
        refused(&["g/Cargo.toml does not hold the root's [workspace.package] table exactly"]),
        "a key after the first table's blank line"
    );
    assert_eq!(
        gate(
            &root.replace("[profile.release]", "[profile.dev]"),
            gpu,
            wgsl
        ),
        refused(&["Cargo.toml has no [workspace.lints.rust] table followed by [profile.release]"]),
        "a root without the table's end"
    );
    assert_eq!(
        gate("", "", ""),
        refused(&[
            "Cargo.toml has no [workspace.package] table followed by [workspace.dependencies]",
            "Cargo.toml has no [workspace.lints.rust] table followed by [profile.release]",
        ]),
        "a root without either table"
    );
}

/// One entry of a lock file as Cargo writes it: `name`, `version`, from the registry with
/// `checksum` (from no source without one), naming `dependencies`.
fn locked(name: &str, version: &str, checksum: &str, dependencies: &[&str]) -> String {
    let mut text = format!("\n[[package]]\nname = \"{name}\"\nversion = \"{version}\"\n");
    if !checksum.is_empty() {
        for line in ["source = \"registry\"\nchecksum = \"", checksum, "\"\n"] {
            text.push_str(line);
        }
    }
    if !dependencies.is_empty() {
        text.push_str("dependencies = [\n");
        for dependency in dependencies {
            for line in [" \"", dependency, "\",\n"] {
                text.push_str(line);
            }
        }
        text.push_str("]\n");
    }
    text
}

/// A lock file of `entries` ([`locked`]).
fn lock_of(entries: &[String]) -> String {
    format!(
        "# This file is automatically @generated by Cargo.\nversion = 4\n{}",
        entries.concat()
    )
}

/// The refusal of the gate on the GPU packages saying `problems`.
fn refused_tree(problems: &[&str]) -> Result<(), Failure> {
    crate::steps::verdict(
        "the WGSL tools build the GPU renderer's naga and wgpu, and the packages beneath them",
        problems.iter().map(|&problem| problem.to_owned()).collect(),
    )
}

/// The renderer's lock of the tests of the gate on the GPU packages, and the entries of
/// half, hexf-parse and naga a tools' lock holds to match it: its naga turns on more
/// features, so more dependencies, which are not compared.
fn renderer_lock() -> (String, [String; 3]) {
    let half = locked("half", "2.7.1", "h1", &[]);
    let hexf = locked("hexf-parse", "0.2.1", "x1", &[]);
    let gpu = lock_of(&[
        half.clone(),
        hexf.clone(),
        locked("naga", "30.0.2", "n1", &["half", "hexf-parse", "spirv"]),
        locked("spirv", "0.3.0", "s1", &[]),
        locked("visual-gpu", "0.6.0", "", &["naga", "wgpu"]),
        locked("wgpu", "30.0.2", "w1", &["naga"]),
    ]);
    let naga = locked("naga", "30.0.2", "n1", &["half", "hexf-parse"]);
    (gpu, [half, hexf, naga])
}

#[test]
fn requires_the_wgsl_tools_to_build_the_gpu_renderers_naga_and_wgpu_and_their_tree() {
    let refused = refused_tree;
    let (gpu, [half, hexf, naga]) = renderer_lock();
    let tool = locked("wgsl-test", "0.6.0", "", &["naga"]);
    let gate = |entries: &[String]| crate::shared_gpu_versions_gate(&gpu, &lock_of(entries));
    assert_eq!(
        gate(&[half.clone(), hexf.clone(), naga.clone(), tool]),
        Ok(()),
        "the same naga and the same packages beneath it, no wgpu yet; the tool itself, from no source, not reached"
    );
    assert_eq!(
        gate(&[
            half.clone(),
            hexf.clone(),
            naga.clone(),
            locked("wgpu", "30.0.2", "w1", &["naga"]),
        ]),
        Ok(()),
        "the same naga and wgpu"
    );
    assert_eq!(
        gate(&[
            locked("half", "2.7.0", "h0", &[]),
            hexf.clone(),
            naga.clone()
        ]),
        refused(&[
            "tooling/wgsl/Cargo.lock resolves half 2.7.0 (registry), which crates/visual-gpu/Cargo.lock does not: 2.7.1 (registry)"
        ]),
        "another version of a package beneath naga"
    );
    assert_eq!(
        gate(&[
            half.clone(),
            hexf.clone(),
            locked("naga", "30.0.2", "", &["half", "hexf-parse"])
        ]),
        refused(&[
            "tooling/wgsl/Cargo.lock resolves naga 30.0.2 (), which crates/visual-gpu/Cargo.lock does not: 30.0.2 (registry)"
        ]),
        "a naga of the same version patched in from a path"
    );
    assert_eq!(
        gate(&[
            half.clone(),
            locked("hexf-parse", "0.2.1", "x2", &[]),
            naga.clone()
        ]),
        refused(&[
            "tooling/wgsl/Cargo.lock resolves hexf-parse 0.2.1 (registry) with another checksum than crates/visual-gpu/Cargo.lock"
        ]),
        "another checksum"
    );
    assert_eq!(
        gate(&[half, hexf, naga, locked("wgpu", "31.0.0", "w2", &[]),]),
        refused(&[
            "tooling/wgsl/Cargo.lock resolves wgpu 31.0.0 (registry), which crates/visual-gpu/Cargo.lock does not: 30.0.2 (registry)"
        ]),
        "another wgpu"
    );
}

#[test]
fn says_a_tree_the_gate_on_the_gpu_packages_cannot_compare() {
    let refused = refused_tree;
    let (gpu, [half, hexf, naga]) = renderer_lock();
    let gate = |entries: &[String]| crate::shared_gpu_versions_gate(&gpu, &lock_of(entries));
    assert_eq!(
        gate(&[half.clone(), locked("wgsl-test", "0.6.0", "", &[])]),
        refused(&["tooling/wgsl/Cargo.lock resolves no naga"]),
        "no naga to compare"
    );
    assert_eq!(
        crate::shared_gpu_versions_gate(&lock_of(&[]), &lock_of(&[half, hexf, naga])),
        refused(&[
            "tooling/wgsl/Cargo.lock resolves naga 30.0.2 (registry), which crates/visual-gpu/Cargo.lock does not: none",
            "tooling/wgsl/Cargo.lock resolves hexf-parse 0.2.1 (registry), which crates/visual-gpu/Cargo.lock does not: none",
            "tooling/wgsl/Cargo.lock resolves half 2.7.1 (registry), which crates/visual-gpu/Cargo.lock does not: none",
        ]),
        "each package reached, said"
    );
    assert_eq!(
        gate(&[locked("naga", "30.0.2", "n1", &["half"])]),
        refused(&["tooling/wgsl/Cargo.lock: a dependency, `half`, names no one entry"]),
        "a dependency the tools' lock does not hold"
    );
    let broken = "version = 4\n\n[[package]]\nname = \"naga\"\n";
    assert!(
        crate::shared_gpu_versions_gate(broken, broken).is_err_and(|failure| {
            failure.details.len() == 2
                && failure
                    .details
                    .iter()
                    .all(|line| line.contains("is not as Cargo writes it"))
        }),
        "an entry without its version, in each lock, said"
    );
    assert_eq!(
        crate::SHARED_GPU_PACKAGES,
        ["naga", "wgpu"],
        "the GPU layer and its shader compiler"
    );
}

#[test]
fn refuses_a_configuration_of_the_tools_below_the_root() {
    let paths =
        |listed: &[&str]| -> Vec<String> { listed.iter().map(|&path| path.to_owned()).collect() };
    assert_eq!(
        crate::nested_tool_configs_gate(&paths(&[
            ".cargo/config.toml",
            "clippy.toml",
            "rust-toolchain.toml",
            "rustfmt.toml",
        ])),
        Ok(()),
        "the root's"
    );
    assert_eq!(
        crate::nested_tool_configs_gate(&paths(&[
            "clippy.toml",
            "tooling/wgsl/clippy.toml",
            "crates/visual-gpu/.cargo/config.toml",
        ])),
        crate::steps::verdict(
            "the tools read their configuration from the root only",
            vec![
                "tooling/wgsl/clippy.toml would replace the root's configuration below it"
                    .to_owned(),
                "crates/visual-gpu/.cargo/config.toml would replace the root's configuration \
                 below it"
                    .to_owned(),
            ],
        ),
        "a workspace apart's own, each said"
    );
    assert_eq!(
        crate::TOOL_CONFIGS,
        [
            "*clippy.toml",
            "*rustfmt.toml",
            "*rust-toolchain",
            "*rust-toolchain.toml",
            "*.cargo/config.toml",
            "*.cargo/config",
            "*.clippy.toml",
        ],
        "Clippy's, rustfmt's, rustup's and Cargo's, each name they read"
    );
}

#[test]
fn names_each_manifest_of_a_workspace_apart_it_cannot_read() {
    for manifest in ["crates/visual-gpu/Cargo.toml", "tooling/wgsl/Cargo.toml"] {
        let mut fake = passing();
        drop(fake.files.remove(manifest));
        assert!(
            crate::actions::run_action(
                &crate::Action::CargoConfigGate,
                &mut fake,
                &mut Vec::new(),
                &mut None,
            )
            .is_err_and(|failure| failure.summary.contains(manifest)),
            "{manifest} unread, named"
        );
    }
}
