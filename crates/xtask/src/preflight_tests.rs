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
    node_configs: Vec::new(),
    vulkan_searched: None,
};

/// What every command's preflight requires: a Vulkan driver on Linux.
const REQUIRED: crate::GpuDriver = crate::GpuDriver::Required;

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
            &CLEAN,
            REQUIRED
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
            preflight_gate("linux", &present(node), pinned, TOOLCHAIN, &CLEAN, REQUIRED),
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
        preflight_gate("windows", &missing, "22.23.4", TOOLCHAIN, &CLEAN, REQUIRED),
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
            preflight_gate(os, &answers, "22.23.4", TOOLCHAIN, &CLEAN, REQUIRED),
            Ok(()),
            "{os}: installed"
        );
    }
    answers[at] = Err(CommandFailure {
        line: "prlimit --version".to_owned(),
        failed: Failed::NotStarted("No such file or directory (os error 2)".to_owned()),
    });
    assert_eq!(
        preflight_gate("macos", &answers, "22.23.4", TOOLCHAIN, &CLEAN, REQUIRED),
        Ok(()),
        "not read off Linux, where the mutation tests run without it"
    );
    assert_eq!(
        preflight_gate("linux", &answers, "22.23.4", TOOLCHAIN, &CLEAN, REQUIRED),
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
    let answers: Vec<Result<String, CommandFailure>> = crate::TOOLS
        .iter()
        .map(|tool| Ok(tool_answer(tool.program, tool.args.first().copied())))
        .collect();
    let missing = crate::Environment {
        vulkan_searched: Some(vec![
            "/etc/vulkan/icd.d".to_owned(),
            "/usr/share/vulkan/icd.d".to_owned(),
        ]),
        ..CLEAN
    };
    assert_eq!(
        preflight_gate("macos", &answers, "22.23.4", TOOLCHAIN, &missing, REQUIRED),
        Ok(()),
        "not read off Linux, where Metal needs no driver to install"
    );
    assert_eq!(
        preflight_gate("linux", &answers, "22.23.4", TOOLCHAIN, &missing, REQUIRED),
        failed(
            "the checks cannot run here: no Vulkan driver declared to the loader (searched \
             /etc/vulkan/icd.d, /usr/share/vulkan/icd.d; VK_DRIVER_FILES, VK_ICD_FILENAMES \
             and VK_ADD_DRIVER_FILES unset); to install: install a Vulkan driver, which the \
             GPU workspace's tests render on (on Ubuntu, mesa-vulkan-drivers: lavapipe renders \
             on the CPU), or name its manifest with VK_DRIVER_FILES if your loader searches \
             other system directories than these"
        ),
        "missing on Linux, with where the loader looked and how to install one"
    );
    assert_eq!(
        preflight_gate("linux", &answers, "22.23.4", TOOLCHAIN, &CLEAN, REQUIRED),
        Ok(()),
        "a driver declared"
    );
    assert_eq!(
        preflight_gate(
            "linux",
            &answers,
            "22.23.4",
            TOOLCHAIN,
            &missing,
            crate::GpuDriver::Unneeded
        ),
        Ok(()),
        "not required where nothing renders on the GPU (the final part of CI)"
    );
}

#[test]
fn looks_for_vulkan_drivers_where_the_loader_does() {
    use std::ffi::OsString;
    let lookup = |pairs: &[(&str, &str)]| {
        let pairs: Vec<(String, OsString)> = pairs
            .iter()
            .map(|(name, value)| ((*name).to_owned(), OsString::from(value)))
            .collect();
        move |name: &str| {
            pairs
                .iter()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value.clone())
        }
    };
    let dirs = |pairs: &[(&str, &str)]| -> Vec<String> {
        crate::preflight::vulkan_driver_dirs(&lookup(pairs))
            .into_iter()
            .map(|dir| dir.display().to_string())
            .collect()
    };
    assert_eq!(
        dirs(&[("HOME", "/home/u")]),
        [
            "/home/u/.config/vulkan/icd.d",
            "/etc/xdg/vulkan/icd.d",
            "/usr/local/etc/vulkan/icd.d",
            "/etc/vulkan/icd.d",
            "/home/u/.local/share/vulkan/icd.d",
            "/usr/local/share/vulkan/icd.d",
            "/usr/share/vulkan/icd.d",
        ],
        "the loader's defaults: the XDG directories, then the system's"
    );
    assert_eq!(
        dirs(&[
            ("XDG_CONFIG_HOME", "/c"),
            ("XDG_CONFIG_DIRS", "/d1:/d2"),
            ("XDG_DATA_HOME", "/h"),
            ("XDG_DATA_DIRS", "/nix/store/x/share"),
        ]),
        [
            "/c/vulkan/icd.d",
            "/d1/vulkan/icd.d",
            "/d2/vulkan/icd.d",
            "/usr/local/etc/vulkan/icd.d",
            "/etc/vulkan/icd.d",
            "/h/vulkan/icd.d",
            "/nix/store/x/share/vulkan/icd.d",
        ],
        "each XDG variable set, no home: as the loader reads them (NixOS's data directories)"
    );
    assert_eq!(
        dirs(&[("XDG_CONFIG_DIRS", ""), ("XDG_DATA_DIRS", "")]),
        [
            "/etc/xdg/vulkan/icd.d",
            "/usr/local/etc/vulkan/icd.d",
            "/etc/vulkan/icd.d",
            "/usr/local/share/vulkan/icd.d",
            "/usr/share/vulkan/icd.d",
        ],
        "an empty list is unset, as XDG says"
    );
    // Which directories hold a manifest, asked of a stand-in: the search never reads this
    // machine's own drivers.
    let searched = |pairs: &[(&str, &str)], holding: Option<&str>| {
        crate::preflight::vulkan_searched(&lookup(pairs), &|dir: &Path| {
            holding.is_some_and(|holding| dir == Path::new(holding))
        })
    };
    let home = [("HOME", "/home/u")];
    assert_eq!(
        searched(&home, None),
        Some(dirs(&home)),
        "no manifest: every directory searched named"
    );
    assert_eq!(
        searched(&home, Some("/home/u/.local/share/vulkan/icd.d")),
        None,
        "a driver's manifest in one of them"
    );
    assert_eq!(
        searched(&home, Some("/opt/vulkan/icd.d")),
        Some(dirs(&home)),
        "a manifest where the loader does not look is none"
    );
    for variable in ["VK_DRIVER_FILES", "VK_ICD_FILENAMES", "VK_ADD_DRIVER_FILES"] {
        assert_eq!(
            searched(
                &[variable, "HOME"].map(|name| (name, "/opt/icd.json")),
                None
            ),
            None,
            "{variable} names a driver to the loader directly"
        );
    }
    assert_eq!(
        searched(&[("VK_DRIVER_FILES", ""), ("HOME", "/home/u")], None),
        Some(dirs(&home)),
        "an empty variable names none"
    );
}

#[test]
fn takes_an_empty_xdg_home_as_unset() {
    let dirs: Vec<String> = crate::preflight::vulkan_driver_dirs(&|name: &str| match name {
        "HOME" => Some(OsString::from("/h")),
        "XDG_CONFIG_HOME" | "XDG_DATA_HOME" => Some(OsString::new()),
        _ => None,
    })
    .into_iter()
    .map(|dir| dir.display().to_string())
    .collect();
    assert_eq!(
        dirs,
        [
            "/h/.config/vulkan/icd.d",
            "/etc/xdg/vulkan/icd.d",
            "/usr/local/etc/vulkan/icd.d",
            "/etc/vulkan/icd.d",
            "/h/.local/share/vulkan/icd.d",
            "/usr/local/share/vulkan/icd.d",
            "/usr/share/vulkan/icd.d",
        ],
        "an empty home is unset, as XDG says: never a directory relative to where xtask \
         runs, which the loader never searches"
    );
}

#[test]
#[expect(
    clippy::disallowed_methods,
    reason = "shows what the ban of DirEntry::file_type in clippy.toml keeps out of the lookup of Vulkan manifests, which also proves the lint fires: the entry's type is the link's, never its target's"
)]
fn a_directory_entry_s_type_is_the_link_s_own() {
    let target = scratch("entry-type-target");
    std::fs::write(target.0.join("driver.json"), "{}").expect("a manifest");
    let dir = scratch("entry-type");
    std::os::unix::fs::symlink(target.0.join("driver.json"), dir.0.join("icd.json"))
        .expect("a link");
    let entry = std::fs::read_dir(&dir.0)
        .expect("the directory")
        .next()
        .expect("one entry")
        .expect("readable");
    let kind = entry.file_type().expect("its type");
    assert!(
        (kind.is_symlink(), kind.is_file(), entry.path().is_file()) == (true, false, true),
        "a link to a file is no file to DirEntry::file_type, one to Path::is_file"
    );
}

#[test]
#[expect(
    clippy::disallowed_methods,
    reason = "shows what the ban of DirEntry::metadata in clippy.toml keeps out of the lookup of Vulkan manifests, which also proves the lint fires: the entry's metadata is the link's, never its target's, a sibling of DirEntry::file_type (finding F17-4 of the fourth deep review of visual block 2)"
)]
fn a_directory_entry_s_metadata_is_the_link_s_own() {
    let target = scratch("entry-metadata-target");
    std::fs::write(target.0.join("driver.json"), "{}").expect("a manifest");
    let dir = scratch("entry-metadata");
    std::os::unix::fs::symlink(target.0.join("driver.json"), dir.0.join("icd.json"))
        .expect("a link");
    let entry = std::fs::read_dir(&dir.0)
        .expect("the directory")
        .next()
        .expect("one entry")
        .expect("readable");
    let kind = entry.metadata().expect("its metadata").file_type();
    assert!(
        (kind.is_symlink(), kind.is_file(), entry.path().is_file()) == (true, false, true),
        "a link to a file is no file to DirEntry::metadata, one to Path::is_file"
    );
}

/// A link to a manifest in a directory of its own, its target in another, both kept while
/// the guards live.
fn linked_manifest(
    name: &str,
) -> (
    crate::tests::Scratch,
    crate::tests::Scratch,
    std::path::PathBuf,
) {
    let target = scratch(&format!("{name}-target"));
    std::fs::write(target.0.join("driver.json"), "{}").expect("a manifest");
    let dir = scratch(name);
    let link = dir.0.join("icd.json");
    std::os::unix::fs::symlink(target.0.join("driver.json"), &link).expect("a link");
    (target, dir, link)
}

#[test]
#[expect(
    clippy::disallowed_methods,
    reason = "shows what the ban of std::fs::symlink_metadata in clippy.toml keeps out, which also proves the lint fires: the metadata is the link's, never its target's, a sibling of DirEntry::metadata (finding H11-5 of the fifth deep review of visual block 2)"
)]
fn a_link_s_metadata_is_its_own() {
    let (_target, _dir, link) = linked_manifest("link-metadata");
    let kind = std::fs::symlink_metadata(&link)
        .expect("its metadata")
        .file_type();
    assert!(
        (kind.is_symlink(), kind.is_file(), link.is_file()) == (true, false, true),
        "a link to a file is no file to symlink_metadata, one to Path::is_file"
    );
}

#[test]
#[expect(
    clippy::disallowed_methods,
    reason = "shows what the ban of Path::symlink_metadata in clippy.toml keeps out, which also proves the lint fires: the metadata is the link's, never its target's, a sibling of DirEntry::metadata (finding H11-5 of the fifth deep review of visual block 2)"
)]
fn a_path_s_link_metadata_is_its_own() {
    let (_target, _dir, link) = linked_manifest("path-link-metadata");
    let kind = link.symlink_metadata().expect("its metadata").file_type();
    assert!(
        (kind.is_symlink(), kind.is_file(), link.is_file()) == (true, false, true),
        "a link to a file is no file to Path::symlink_metadata, one to Path::is_file"
    );
}

#[test]
fn reads_a_vulkan_driver_directory_for_its_manifests() {
    use crate::preflight::holds_a_manifest;
    let dir = scratch("vulkan");
    assert!(!holds_a_manifest(&dir.0.join("absent")), "no directory");
    std::fs::write(dir.0.join("notes.txt"), "").expect("not a manifest");
    std::fs::create_dir_all(dir.0.join("nested.json")).expect("a directory named alike");
    assert!(!holds_a_manifest(&dir.0), "no manifest file");
    std::fs::write(dir.0.join("lvp_icd.x86_64.json"), "{}").expect("a manifest");
    assert!(holds_a_manifest(&dir.0), "a driver's manifest");
    // The loader opens each manifest by its path (`fopen`), which follows links: a
    // directory of links to manifests elsewhere (NixOS, Guix, a driver managed by links)
    // declares drivers, a dangling link or a link to a directory does not.
    let target = scratch("vulkan-target");
    std::fs::write(target.0.join("driver.json"), "{}").expect("a manifest elsewhere");
    let links = |name: &str, to: &Path| {
        let linking = scratch(name);
        std::os::unix::fs::symlink(to, linking.0.join("icd.json")).expect("a link");
        holds_a_manifest(&linking.0)
    };
    assert!(
        links("vulkan-linked", &target.0.join("driver.json")),
        "a link to a manifest"
    );
    assert!(
        !links("vulkan-dangling", &target.0.join("absent.json")),
        "a dangling link"
    );
    assert!(
        !links("vulkan-to-dir", &target.0),
        "a link to a directory named as a manifest"
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
            &CLEAN,
            REQUIRED
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
            preflight_gate(
                "linux",
                &answers(rustc),
                "22.23.4",
                toolchain,
                &CLEAN,
                REQUIRED
            ),
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
                &CLEAN,
                REQUIRED
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
        node_configs: Vec::new(),
        vulkan_searched: None,
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
        preflight_gate("linux", &answers, "22.23.4", TOOLCHAIN, &hostile, REQUIRED),
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
fn lists_the_npm_and_pnpm_configurations_of_the_user_and_the_machine() {
    let configs = |home: Option<&str>, xdg: Option<&str>, path: Option<&str>| -> Vec<String> {
        crate::node_config_candidates(
            home.map(OsString::from),
            xdg.map(OsString::from),
            path.map(OsString::from).as_deref(),
        )
        .iter()
        .map(|candidate| candidate.display().to_string())
        .collect()
    };
    assert_eq!(
        configs(Some("/home/x"), None, Some("/opt/node/bin:/usr/bin")),
        [
            "/home/x/.config/pnpm/rc",
            "/home/x/.npmrc",
            "/home/x/Library/Preferences/pnpm/rc",
            "/opt/node/etc/npmrc",
            "/usr/etc/npmrc",
        ],
        "the user's npm and pnpm files, and the global npm file beside each directory of \
         programs (npm's prefix holds Node.js)"
    );
    assert_eq!(
        configs(Some("/home/x"), Some("/xdg"), None),
        [
            "/home/x/.npmrc",
            "/home/x/Library/Preferences/pnpm/rc",
            "/xdg/pnpm/rc"
        ],
        "pnpm's below XDG_CONFIG_HOME when set"
    );
    assert_eq!(
        configs(None, None, Some("/")),
        Vec::<String>::new(),
        "no home, a root"
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
        preflight_gate("linux", &answers, "22.23.4", TOOLCHAIN, &CLEAN, REQUIRED),
        Ok(()),
        "the pinned versions"
    );
    let mut older = answers;
    older[5] = Ok("cargo-mutants 26.0.0\n".to_owned());
    older[6] = Ok("cargo-deny".to_owned());
    assert_eq!(
        preflight_gate("linux", &older, "22.23.4", TOOLCHAIN, &CLEAN, REQUIRED),
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
            preflight_gate("linux", &answers, text, TOOLCHAIN, &CLEAN, REQUIRED).is_ok(),
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

/// The problem of a mutant left at `place`, in the file whose path quoted for a shell,
/// its quotes escaped, is `quoted`, with the hint restoring it.
fn restore(place: &str, quoted: &str) -> String {
    format!(
        "{place} holds cargo-mutants' marker: restore it when it holds no change of yours \
         (see: git restore --worktree -- ':/{quoted}')"
    )
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
        failed(format!(
            "a mutant left in place by a stopped mutation run (each file named restores from \
             the index, which keeps what you staged, by the hint after it): {}",
            restore(
                "crates/visual-gpu/src/band.rs:2",
                "crates/visual-gpu/src/band.rs"
            )
        )),
        "the file and the line named, with how to restore it"
    );
    assert_eq!(
        mutant_left_gate(&format!(
            "{found}crates/visual-gpu/tests/a.rs:5:{MUTANT_MARKER}: a colon\nodd\n"
        ))
        .map_err(|failure| failure.details),
        Err(vec![
            restore(
                "crates/visual-gpu/src/band.rs:2",
                "crates/visual-gpu/src/band.rs"
            ),
            restore(
                "crates/visual-gpu/tests/a.rs:5",
                "crates/visual-gpu/tests/a.rs"
            ),
            "odd holds cargo-mutants' marker".to_owned(),
        ]),
        "every line named, a line grep would never write named whole, with no hint"
    );
    assert_eq!(
        mutant_left_gate(&format!("crates/visual-gpu/it's.rs:1:{MUTANT_MARKER}\n"))
            .map_err(|failure| failure.summary),
        Err(format!(
            "a mutant left in place by a stopped mutation run (each file named restores from \
             the index, which keeps what you staged, by the hint after it): {}",
            restore(
                "crates/visual-gpu/it's.rs:1",
                r"crates/visual-gpu/it'\''s.rs"
            )
        )),
        "a quote in the path escaped for the shell"
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

#[test]
fn refuses_what_changes_how_a_runtime_runs_the_checks() {
    // Node.js's options reach every TypeScript check and the processes its tests spawn: a
    // preload or an inspector would change a run from outside the repository. Findings
    // F1-1, F11-3 and F17-3 of the fourth deep review of visual block 2 (#34), as the
    // maintainer decided: so do Node.js's mode and module paths, and every setting npm and
    // pnpm read from a variable, in any case.
    for name in [
        "NODE_OPTIONS",
        "NODE_ENV",
        "NODE_PATH",
        "NODE_PRESERVE_SYMLINKS",
        "npm_config_registry",
        "NPM_CONFIG_NODE_LINKER",
        "pnpm_config_store_dir",
        "PNPM_CONFIG_x",
    ] {
        assert!(crate::refused_variable(name), "{name}");
    }
    for name in [
        "NODE_OPTIONSX",
        "XNODE_OPTIONS",
        "node_options",
        "NODE_VERSION",
        "npm_config",
        "xnpm_config_a",
        "PNPM_HOME",
    ] {
        assert!(!crate::refused_variable(name), "{name}");
    }
    let answers: Vec<Result<String, CommandFailure>> = crate::TOOLS
        .iter()
        .map(|tool| Ok(tool_answer(tool.program, tool.args.first().copied())))
        .collect();
    let environment = crate::Environment {
        variables: vec![
            "NODE_OPTIONS".to_owned(),
            "NODE_ENV".to_owned(),
            "npm_config_registry".to_owned(),
            "RUSTFLAGS".to_owned(),
        ],
        cargo_configs: Vec::new(),
        node_configs: vec!["/home/x/.npmrc".to_owned()],
        vulkan_searched: None,
    };
    assert_eq!(
        preflight_gate(
            "linux",
            &answers,
            "22.23.4",
            TOOLCHAIN,
            &environment,
            REQUIRED
        ),
        found(
            "the checks cannot run here",
            &[
                "NODE_OPTIONS is set, which changes how Node.js runs the TypeScript checks and \
                 the processes their tests start (a preloaded module, an inspector, another \
                 flag): unset it"
                    .to_owned(),
                "NODE_ENV is set, which changes how Node.js runs the TypeScript checks and the \
                 processes their tests start (the mode tools and packages pick, production or \
                 development): unset it"
                    .to_owned(),
                "npm_config_registry is set, which changes how npm runs the TypeScript checks \
                 and the processes their tests start (a setting of every npm and pnpm command, \
                 read in any case): unset it"
                    .to_owned(),
                "RUSTFLAGS is set, which changes what cargo builds or how it lints: unset it"
                    .to_owned(),
                "/home/x/.npmrc configures every npm or pnpm command here: move it away (the \
                 configuration of the repository is pnpm-workspace.yaml)"
                    .to_owned(),
            ]
        )
    );
    let mut fake = passing();
    fake.environment.variables = vec!["NODE_OPTIONS".to_owned()];
    assert_eq!(
        run_steps(&check_steps(CHANGES_MODE), &mut fake, &mut Vec::new()),
        failed(
            "Tools and platform: the checks cannot run here: NODE_OPTIONS is set, which changes \
             how Node.js runs the TypeScript checks and the processes their tests start (a \
             preloaded module, an inspector, another flag): unset it"
        ),
        "the check reads the environment of its runner"
    );
}
