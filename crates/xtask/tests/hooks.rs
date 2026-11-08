//! The git hooks, run by real git: what reaches `cargo xtask` when a push or a commit
//! starts from a linked worktree, as the CI section of `CLAUDE.md` recommends.

#![forbid(unsafe_code)]
#![cfg(unix)]

#[cfg(test)]
mod tests {
    use std::fs;
    use std::os::unix::fs::PermissionsExt as _;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    use xtask::LOCAL_GIT_VARIABLES;

    /// The variables of `received` (the stub's record, one `NAME=value` per line) that describe a
    /// repository: none may reach the check.
    fn local_variables(received: &str) -> Vec<&str> {
        received
            .lines()
            .filter_map(|line| line.split_once('=').map(|(name, _)| name))
            .filter(|name| LOCAL_GIT_VARIABLES.contains(name))
            .collect()
    }

    /// A fresh directory of this test, emptied first.
    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("xtask-hooks-{name}-{}", std::process::id()));
        if dir.exists() {
            fs::remove_dir_all(&dir).expect("old scratch removed");
        }
        fs::create_dir_all(&dir).expect("scratch created");
        dir
    }

    /// Runs git in `dir`, hermetically: no variable of an enclosing git process leaks in.
    fn git(dir: &Path, args: &[&str], path: &str) {
        let mut command = Command::new("git");
        let _same = command
            .args(args)
            .current_dir(dir)
            .env("PATH", path)
            .env("GIT_AUTHOR_NAME", "Test")
            .env("GIT_AUTHOR_EMAIL", "test@example.invalid")
            .env("GIT_COMMITTER_NAME", "Test")
            .env("GIT_COMMITTER_EMAIL", "test@example.invalid")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("HOME", dir);
        for variable in LOCAL_GIT_VARIABLES {
            let _without = command.env_remove(variable);
        }
        let status = command.status().expect("git runs");
        assert!(status.success(), "git {args:?} in {}", dir.display());
    }

    /// A stand-in for `cargo` that writes the git variables it received to `record`.
    fn stub_cargo(bin: &Path, record: &Path) {
        fs::create_dir_all(bin).expect("bin created");
        let script = bin.join("cargo");
        let body = format!(
            "#!/bin/sh\nenv | grep '^GIT_' | grep -v -e '^GIT_AUTHOR' -e '^GIT_COMMITTER' -e '^GIT_CONFIG_NOSYSTEM' > '{}'\nexit 0\n",
            record.display()
        );
        fs::write(&script, body).expect("stub written");
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).expect("stub executable");
    }

    /// The repository's own hooks, committed in `repo` under `.githooks` and enabled through
    /// `core.hooksPath`, as in the project: a relative hooks path is resolved in each worktree.
    fn install_hooks(repo: &Path, path: &str) {
        let hooks = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.githooks");
        let installed = repo.join(".githooks");
        fs::create_dir_all(&installed).expect("hooks dir");
        for hook in ["pre-push", "commit-msg"] {
            let target = installed.join(hook);
            let _bytes = fs::copy(hooks.join(hook), &target).expect("hook copied");
            fs::set_permissions(&target, fs::Permissions::from_mode(0o755))
                .expect("hook executable");
        }
        git(repo, &["config", "core.hooksPath", ".githooks"], path);
    }

    #[test]
    fn a_push_from_a_linked_worktree_passes_no_git_variable_to_the_check() {
        let root = scratch("push");
        let record = root.join("record.txt");
        let bin = root.join("bin");
        stub_cargo(&bin, &record);
        let path = format!(
            "{}:/usr/bin:/bin:/usr/local/bin:/opt/homebrew/bin",
            bin.display()
        );
        let main = root.join("main");
        fs::create_dir_all(&main).expect("main created");
        git(&main, &["init", "-q", "-b", "main"], &path);
        install_hooks(&main, &path);
        fs::write(main.join("a.txt"), "a\n").expect("file written");
        git(&main, &["add", "a.txt", ".githooks"], &path);
        git(
            &main,
            &["commit", "-q", "--no-verify", "-m", "docs: start"],
            &path,
        );
        let remote = root.join("remote.git");
        git(
            &root,
            &["init", "-q", "--bare", remote.to_str().expect("utf-8")],
            &path,
        );
        let worktree = root.join("worktree");
        git(
            &main,
            &[
                "worktree",
                "add",
                "-q",
                worktree.to_str().expect("utf-8"),
                "-b",
                "work",
            ],
            &path,
        );
        git(
            &worktree,
            &["push", "-q", remote.to_str().expect("utf-8"), "work"],
            &path,
        );
        let received = fs::read_to_string(&record).expect("the hook ran cargo");
        assert_eq!(local_variables(&received), Vec::<&str>::new(), "{received}");
        fs::remove_dir_all(&root).expect("scratch removed");
    }

    #[test]
    fn a_commit_from_a_linked_worktree_passes_no_git_variable_to_the_message_check() {
        let root = scratch("commit");
        let record = root.join("record.txt");
        let bin = root.join("bin");
        stub_cargo(&bin, &record);
        let path = format!(
            "{}:/usr/bin:/bin:/usr/local/bin:/opt/homebrew/bin",
            bin.display()
        );
        let main = root.join("main");
        fs::create_dir_all(&main).expect("main created");
        git(&main, &["init", "-q", "-b", "main"], &path);
        install_hooks(&main, &path);
        fs::write(main.join("a.txt"), "a\n").expect("file written");
        git(&main, &["add", "a.txt", ".githooks"], &path);
        git(
            &main,
            &["commit", "-q", "--no-verify", "-m", "docs: start"],
            &path,
        );
        let worktree = root.join("worktree");
        git(
            &main,
            &[
                "worktree",
                "add",
                "-q",
                worktree.to_str().expect("utf-8"),
                "-b",
                "work",
            ],
            &path,
        );
        fs::write(worktree.join("b.txt"), "b\n").expect("file written");
        git(&worktree, &["add", "b.txt"], &path);
        git(&worktree, &["commit", "-q", "-m", "docs: more"], &path);
        let received = fs::read_to_string(&record).expect("the hook ran cargo");
        assert_eq!(local_variables(&received), Vec::<&str>::new(), "{received}");
        fs::remove_dir_all(&root).expect("scratch removed");
    }
}
