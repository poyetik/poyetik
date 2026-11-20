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

/// A root manifest holding the tables the GPU workspace's shares.
const ROOT_MANIFEST: &str = "[workspace]\n\n[workspace.package]\nversion = \"1.0.0\"\n\n[workspace.dependencies]\nx = 1\n\n[workspace.lints.rust]\nwarnings = \"deny\"\n\n[workspace.lints.rustdoc]\nbare_urls = \"deny\"\n\n[workspace.lints.clippy]\nall = \"deny\"\n\n[profile.release]\nlto = true\n";

/// A GPU workspace's manifest the gate accepts against [`ROOT_MANIFEST`].
const GPU_MANIFEST_TEXT: &str = "# Why.\n[workspace]\nresolver = \"3\"\nmembers = [\".\"]\n\n[workspace.package]\nversion = \"1.0.0\"\n\n[workspace.lints.rust]\nwarnings = \"deny\"\n\n[workspace.lints.rustdoc]\nbare_urls = \"deny\"\n\n[workspace.lints.clippy]\nall = \"deny\"\n\n[package]\nname = \"g\"\n\n[dependencies]\nwgpu = \"1\"\n\n[dev-dependencies]\n\n[lints]\nworkspace = true\n";

#[test]
fn requires_the_gpu_workspace_to_share_the_roots_settings() {
    let (root, gpu) = (ROOT_MANIFEST, GPU_MANIFEST_TEXT);
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
fn refuses_a_source_left_mutated_by_a_stopped_run() {
    use crate::preflight::{MUTANT_MARKER, mutant_left_gate, mutant_search_command};
    assert_eq!(
        MUTANT_MARKER, "/* ~ changed by cargo-mutants ~ */",
        "the marker of the pinned cargo-mutants, after every span it mutates"
    );
    assert_eq!(
        mutant_search_command().line(),
        "grep -rnF --include=*.rs --exclude-dir=target -e /* ~ changed by cargo-mutants ~ */ -- \
         crates/visual-gpu",
        "the GPU workspace's Rust files on disk, tracked or not, its build apart"
    );
    // As grep names a line holding it: `<file>:<line>:<text>`.
    let found = format!("crates/visual-gpu/src/band.rs:2:    0 {MUTANT_MARKER}\n");
    assert_eq!(
        mutant_left_gate(&found),
        failed(
            "a mutant left in place by a stopped mutation run (restore each file named, with \
             `git checkout -- <file>` when it holds no change of yours): \
             crates/visual-gpu/src/band.rs:2 holds cargo-mutants' marker"
        ),
        "the file and the line named, with how to restore it"
    );
    assert_eq!(
        mutant_left_gate(&format!(
            "{found}crates/visual-gpu/tests/a.rs:5:{MUTANT_MARKER}: a colon\nodd\n"
        ))
        .map_err(|failure| failure.details),
        Err(vec![
            "crates/visual-gpu/src/band.rs:2 holds cargo-mutants' marker".to_owned(),
            "crates/visual-gpu/tests/a.rs:5 holds cargo-mutants' marker".to_owned(),
            "odd holds cargo-mutants' marker".to_owned(),
        ]),
        "every line named, a line grep would never write named whole"
    );
    assert_eq!(mutant_left_gate(""), Ok(()), "none found");
    assert_eq!(mutant_left_gate("\n  \n"), Ok(()), "blank lines name none");
}

#[test]
fn refuses_keys_before_the_gpu_manifests_first_table() {
    let refused = Err(Failure::from(
        "the GPU workspace shares the root's package fields and lints: \
         crates/visual-gpu/Cargo.toml holds more than comments and blank lines before its \
         first table, [workspace]"
            .to_owned(),
    ));
    for (opening, what) in [
        (
            "patch.crates-io.wgpu = { path = \"../w\" }\n",
            "a top-level dotted key patching a dependency",
        ),
        (
            "\n# Why.\nprofile.test.overflow-checks = false\n\n",
            "a top-level dotted key setting a profile, below a comment",
        ),
    ] {
        assert_eq!(
            crate::gpu_manifest_gate(ROOT_MANIFEST, &format!("{opening}{GPU_MANIFEST_TEXT}")),
            refused,
            "{what}"
        );
    }
    assert_eq!(
        crate::gpu_manifest_gate(
            ROOT_MANIFEST,
            &format!("  # Indented.\n\n{GPU_MANIFEST_TEXT}")
        ),
        Ok(()),
        "comments and blank lines only"
    );
}
