//! Finding the checker binary: the pinned `tsrs` installed with Caliper, else the one the project installed.

use camino::{Utf8Path, Utf8PathBuf};

/// The `tsrs` build Caliper pins (ADR 0026) and installs beside its binaries as `@tsrs/<platform>`. The file it is read
/// from is the one place that states the pin: `scripts/check-packaging.mjs` holds every npm manifest that names `tsrs`
/// to it.
pub(crate) const TSRS_VERSION: &str = include_str!("../TSRS_VERSION").trim_ascii();

/// The binaries searched for in each `node_modules` above the project, in order of precedence: `typescript@7.1`'s
/// `tsc`, then the `tsrs` Node shim. An install that has both keeps using `typescript`, as before `tsrs` was supported;
/// `--tsc-path` picks the other.
const CANDIDATES: [[&str; 3]; 2] = [["typescript", "bin", "tsc"], ["tsrs", "bin", "tsrs"]];

/// The binary `tsrs` publishes for one target: `node_modules/@tsrs/<package>/<exe>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TsrsPlatform {
    package: &'static str,
    exe: &'static str,
}

impl TsrsPlatform {
    /// The package for a Rust `target_os`, `target_arch` and `target_env`, or why there is none.
    pub(crate) fn of(os: &str, arch: &str, env: &str) -> Result<Self, String> {
        let package = match (os, arch, env) {
            ("macos", "aarch64", _) => "darwin-arm64",
            ("macos", "x86_64", _) => "darwin-x64",
            ("linux", "x86_64", "gnu") => "linux-x64-gnu",
            ("linux", "x86_64", "musl") => "linux-x64-musl",
            ("linux", "aarch64", "gnu") => "linux-arm64-gnu",
            ("linux", "aarch64", "musl") => "linux-arm64-musl",
            ("windows", "x86_64", _) => "win32-x64",
            _ => return Err(format!("tsrs publishes no binary for {os}-{arch}")),
        };
        Ok(Self { package, exe: if os == "windows" { "tsrs.exe" } else { "tsrs" } })
    }

    /// The package for the target this binary was built for. Its libc is the host's: a glibc build does not run on
    /// musl, nor a musl build on glibc.
    fn host() -> Result<Self, String> {
        let env = if cfg!(target_env = "musl") {
            "musl"
        } else if cfg!(target_env = "gnu") {
            "gnu"
        } else {
            ""
        };
        Self::of(std::env::consts::OS, std::env::consts::ARCH, env)
    }
}

/// The checker to run, in this order:
///
/// 1. `explicit` (`--tsc-path`, `tscPath`), which must exist;
/// 2. the pinned `@tsrs/<platform>` in the nearest `node_modules` at or above `binary_dir`, the directory of the running
///    Caliper binary (the CLI executable or the napi addon), where its npm package installs it;
/// 3. the nearest `node_modules` at or above `project_root` that holds `typescript/bin/tsc`, else `tsrs/bin/tsrs`.
///
/// The version is checked later, when the process is spawned. Finding none is an error naming every place it looked.
pub(crate) fn find_tsc(
    project_root: &Utf8Path,
    explicit: Option<&Utf8Path>,
    binary_dir: Result<&Utf8Path, &str>,
) -> Result<Utf8PathBuf, String> {
    find_tsc_on(TsrsPlatform::host(), project_root, explicit, binary_dir)
}

fn find_tsc_on(
    platform: Result<TsrsPlatform, String>,
    project_root: &Utf8Path,
    explicit: Option<&Utf8Path>,
    binary_dir: Result<&Utf8Path, &str>,
) -> Result<Utf8PathBuf, String> {
    if let Some(path) = explicit {
        return if path.is_file() { Ok(path.to_owned()) } else { Err(format!("no tsc binary at {path}")) };
    }
    if let (Ok(platform), Ok(dir)) = (&platform, binary_dir) {
        let installed = dir
            .ancestors()
            .map(|dir| dir.join("node_modules/@tsrs").join(platform.package).join(platform.exe))
            .find(|candidate| candidate.is_file());
        if let Some(tsrs) = installed {
            return Ok(tsrs);
        }
    }
    let in_project = project_root
        .ancestors()
        .flat_map(|dir| {
            CANDIDATES.map(|parts| parts.iter().fold(dir.join("node_modules"), |path, part| path.join(part)))
        })
        .find(|candidate| candidate.is_file());
    in_project.ok_or_else(|| {
        let (missing, reinstall) = match (platform, binary_dir) {
            (Ok(platform), Ok(dir)) => (
                format!(
                    "the pinned @tsrs/{}@{TSRS_VERSION} is not installed in a node_modules at or above {dir}",
                    platform.package
                ),
                "reinstall Caliper with its optional dependencies (not --omit=optional or --no-optional), ",
            ),
            (Ok(platform), Err(unknown)) => (
                format!(
                    "the pinned @tsrs/{}@{TSRS_VERSION} was not looked for, as the location of Caliper's binary is \
                     unknown ({unknown})",
                    platform.package
                ),
                "",
            ),
            (Err(reason), _) => (reason, ""),
        };
        format!(
            "no TypeScript checker: {missing}, and there is no node_modules/typescript/bin/tsc or \
             node_modules/tsrs/bin/tsrs at or above {project_root}; {reinstall}install `tsrs` or `typescript@beta` \
             (7.1), or pass --tsc-path"
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const DARWIN_ARM64: TsrsPlatform = TsrsPlatform { package: "darwin-arm64", exe: "tsrs" };

    fn tree() -> (tempfile::TempDir, Utf8PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let root = Utf8PathBuf::from_path_buf(dir.path().to_path_buf()).unwrap();
        (dir, root)
    }

    fn touch(path: &Utf8Path) -> Utf8PathBuf {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, "").unwrap();
        path.to_owned()
    }

    fn find(
        project: &Utf8Path,
        explicit: Option<&Utf8Path>,
        binary_dir: Option<&Utf8Path>,
    ) -> Result<Utf8PathBuf, String> {
        find_tsc_on(Ok(DARWIN_ARM64), project, explicit, binary_dir.ok_or("no binary directory was given"))
    }

    #[test]
    fn each_target_maps_to_the_package_tsrs_publishes_for_it() {
        let cases = [
            (("macos", "aarch64", ""), Ok(("darwin-arm64", "tsrs"))),
            (("macos", "x86_64", ""), Ok(("darwin-x64", "tsrs"))),
            (("linux", "x86_64", "gnu"), Ok(("linux-x64-gnu", "tsrs"))),
            (("linux", "x86_64", "musl"), Ok(("linux-x64-musl", "tsrs"))),
            (("linux", "aarch64", "gnu"), Ok(("linux-arm64-gnu", "tsrs"))),
            (("linux", "aarch64", "musl"), Ok(("linux-arm64-musl", "tsrs"))),
            (("windows", "x86_64", "msvc"), Ok(("win32-x64", "tsrs.exe"))),
            (("windows", "aarch64", "msvc"), Err("tsrs publishes no binary for windows-aarch64")),
            (("freebsd", "x86_64", ""), Err("tsrs publishes no binary for freebsd-x86_64")),
        ];
        for ((os, arch, env), expected) in cases {
            let found = TsrsPlatform::of(os, arch, env).map(|platform| (platform.package, platform.exe));
            assert_eq!(found, expected.map_err(str::to_owned), "{os} {arch} {env}");
        }
    }

    #[test]
    fn the_pin_is_read_without_its_line_ending_and_is_a_supported_version() {
        assert_eq!(TSRS_VERSION, include_str!("../TSRS_VERSION").trim_end_matches('\n'));
        assert!(!TSRS_VERSION.contains(char::is_whitespace), "{TSRS_VERSION:?}");
        assert_eq!(crate::SUPPORTED_VERSIONS, ["7.1", TSRS_VERSION]);
    }

    #[test]
    fn the_tsrs_installed_beside_an_npm_hoisted_binary_is_found() {
        let (_dir, root) = tree();
        let binary_dir = root.join("node_modules/caliper-docgen-darwin-arm64");
        let tsrs = touch(&root.join("node_modules/@tsrs/darwin-arm64/tsrs"));

        assert_eq!(find(&root.join("src"), None, Some(&binary_dir)), Ok(tsrs));
    }

    #[test]
    fn the_tsrs_installed_beside_a_pnpm_binary_is_found_in_its_store_directory() {
        let (_dir, root) = tree();
        let store = root.join("node_modules/.pnpm/caliper-docgen-darwin-arm64@0.10.0/node_modules");
        let binary_dir = store.join("caliper-docgen-darwin-arm64");
        let tsrs = touch(&store.join("@tsrs/darwin-arm64/tsrs"));

        assert_eq!(find(&root.join("src"), None, Some(&binary_dir)), Ok(tsrs));
    }

    #[test]
    fn the_nearest_installed_tsrs_wins_over_a_hoisted_one() {
        let (_dir, root) = tree();
        let binary_dir = root.join("node_modules/caliper-docgen-darwin-arm64");
        touch(&root.join("node_modules/@tsrs/darwin-arm64/tsrs"));
        let nested = touch(&binary_dir.join("node_modules/@tsrs/darwin-arm64/tsrs"));

        assert_eq!(find(&root, None, Some(&binary_dir)), Ok(nested));
    }

    #[test]
    fn the_installed_tsrs_wins_over_the_projects_typescript() {
        let (_dir, root) = tree();
        touch(&root.join("node_modules/typescript/bin/tsc"));
        let tsrs = touch(&root.join("node_modules/@tsrs/darwin-arm64/tsrs"));

        assert_eq!(find(&root, None, Some(&root.join("node_modules/caliper-docgen-darwin-arm64"))), Ok(tsrs));
    }

    #[test]
    fn an_explicit_path_wins_over_the_installed_tsrs() {
        let (_dir, root) = tree();
        touch(&root.join("node_modules/@tsrs/darwin-arm64/tsrs"));
        let tsc = touch(&root.join("node_modules/typescript/bin/tsc"));

        assert_eq!(find(&root, Some(&tsc), Some(&root.join("node_modules/caliper-docgen-darwin-arm64"))), Ok(tsc));
    }

    #[test]
    fn another_platforms_tsrs_is_not_taken() {
        let (_dir, root) = tree();
        touch(&root.join("node_modules/@tsrs/linux-x64-gnu/tsrs"));
        let tsc = touch(&root.join("node_modules/typescript/bin/tsc"));

        assert_eq!(find(&root, None, Some(&root.join("node_modules/caliper-docgen-darwin-arm64"))), Ok(tsc));
    }

    #[test]
    fn musl_and_windows_find_their_own_package_and_file() {
        let (_dir, root) = tree();
        let binary_dir = root.join("node_modules/caliper-docgen-x");
        touch(&root.join("node_modules/@tsrs/linux-x64-gnu/tsrs"));
        let musl = touch(&root.join("node_modules/@tsrs/linux-x64-musl/tsrs"));
        touch(&root.join("node_modules/@tsrs/win32-x64/tsrs"));
        let windows = touch(&root.join("node_modules/@tsrs/win32-x64/tsrs.exe"));

        let on = |os, arch, env| find_tsc_on(TsrsPlatform::of(os, arch, env), &root, None, Ok(&binary_dir));
        assert_eq!(on("linux", "x86_64", "musl"), Ok(musl));
        assert_eq!(on("windows", "x86_64", "msvc"), Ok(windows));
    }

    #[test]
    fn a_missing_installed_tsrs_falls_back_to_the_projects_checker() {
        let (_dir, root) = tree();
        let tsc = touch(&root.join("node_modules/typescript/bin/tsc"));

        assert_eq!(find(&root, None, Some(&root.join("node_modules/caliper-docgen-darwin-arm64"))), Ok(tsc));
    }

    #[test]
    fn the_nearest_installed_typescript_above_the_project_wins() {
        let (_dir, root) = tree();
        let tsc = touch(&root.join("node_modules/typescript/bin/tsc"));

        assert_eq!(find(&root.join("packages/ui"), None, None), Ok(tsc));
    }

    #[test]
    fn the_tsrs_shim_is_found_when_typescript_is_not_installed() {
        let (_dir, root) = tree();
        let shim = touch(&root.join("node_modules/tsrs/bin/tsrs"));

        assert_eq!(find(&root.join("packages/ui"), None, None), Ok(shim));
    }

    #[test]
    fn typescript_wins_over_tsrs_in_the_same_node_modules() {
        let (_dir, root) = tree();
        touch(&root.join("node_modules/tsrs/bin/tsrs"));
        let tsc = touch(&root.join("node_modules/typescript/bin/tsc"));

        assert_eq!(find(&root, None, None), Ok(tsc));
    }

    #[test]
    fn the_nearest_node_modules_wins_whichever_checker_it_holds() {
        let (_dir, root) = tree();
        touch(&root.join("node_modules/typescript/bin/tsc"));
        let near = touch(&root.join("packages/ui/node_modules/tsrs/bin/tsrs"));

        assert_eq!(find(&root.join("packages/ui/src"), None, None), Ok(near));
    }

    #[test]
    fn a_project_without_a_checker_says_where_it_looked_and_what_to_install() {
        let (_dir, root) = tree();
        let binary_dir = root.join("node_modules/caliper-docgen-darwin-arm64");

        assert_eq!(
            find(&root.join("src"), None, Some(&binary_dir)),
            Err(format!(
                "no TypeScript checker: the pinned @tsrs/darwin-arm64@{TSRS_VERSION} is not installed in a \
                 node_modules at or above {binary_dir}, and there is no node_modules/typescript/bin/tsc or \
                 node_modules/tsrs/bin/tsrs at or above {src}; reinstall Caliper with its optional dependencies (not \
                 --omit=optional or --no-optional), install `tsrs` or `typescript@beta` (7.1), or pass --tsc-path",
                src = root.join("src"),
            ))
        );
    }

    #[test]
    fn a_platform_tsrs_does_not_publish_for_says_so() {
        let (_dir, root) = tree();

        let error = find_tsc_on(TsrsPlatform::of("windows", "aarch64", "msvc"), &root, None, Ok(&root)).unwrap_err();

        assert_eq!(
            error,
            format!(
                "no TypeScript checker: tsrs publishes no binary for windows-aarch64, and there is no \
                 node_modules/typescript/bin/tsc or node_modules/tsrs/bin/tsrs at or above {root}; install `tsrs` or \
                 `typescript@beta` (7.1), or pass --tsc-path"
            )
        );
    }

    #[test]
    fn a_binary_whose_location_is_unknown_says_the_installed_tsrs_was_not_looked_for() {
        let (_dir, root) = tree();

        assert_eq!(
            find(&root, None, None),
            Err(format!(
                "no TypeScript checker: the pinned @tsrs/darwin-arm64@{TSRS_VERSION} was not looked for, as the \
                 location of Caliper's binary is unknown (no binary directory was given), and there is no \
                 node_modules/typescript/bin/tsc or node_modules/tsrs/bin/tsrs at or above {root}; install `tsrs` or \
                 `typescript@beta` (7.1), or pass --tsc-path"
            ))
        );
    }

    #[test]
    fn an_explicit_path_that_does_not_exist_is_an_error_not_a_search() {
        let (_dir, root) = tree();
        touch(&root.join("node_modules/@tsrs/darwin-arm64/tsrs"));

        assert_eq!(
            find(&root, Some(&root.join("nope")), Some(&root)),
            Err(format!("no tsc binary at {}", root.join("nope")))
        );
    }
}
