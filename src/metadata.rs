use anyhow::{anyhow, bail, Context as _};
use cargo_metadata::{Metadata, MetadataCommand, Package, Resolve, Target};
use serde::Deserialize;
use std::{
    collections::BTreeMap,
    env, fs,
    path::{Path, PathBuf},
    process::Command,
    str,
};
use url::Url;

pub(crate) fn cargo_locate_project(
    manifest_path: Option<&Path>,
    cwd: &Path,
) -> anyhow::Result<PathBuf> {
    let output = Command::new(env::var_os("CARGO").with_context(|| "`$CARGO` should be present")?)
        .arg("locate-project")
        .args(
            manifest_path
                .map(|p| vec!["--manifest-path".as_ref(), p.as_os_str()])
                .unwrap_or_default(),
        )
        .current_dir(cwd)
        .output()?;

    let stdout = str::from_utf8(&output.stdout)?.trim_end();
    let stderr = str::from_utf8(&output.stderr)?.trim_end();

    if !output.status.success() {
        bail!("{}", stderr.trim_start_matches("error: "));
    }

    let ProjectLocation { root } = serde_json::from_str(stdout)?;
    return Ok(root);

    #[derive(Deserialize)]
    struct ProjectLocation {
        root: PathBuf,
    }
}

pub(crate) fn cargo_metadata(manifest_path: Option<&Path>, cwd: &Path) -> anyhow::Result<Metadata> {
    // with `--no-deps`, `cargo metadata` does not update the lockfile properly.
    let mut cmd = MetadataCommand::new();
    if let Some(manifest_path) = manifest_path {
        cmd.manifest_path(manifest_path);
    }
    cmd.current_dir(cwd).exec().map_err(|err| match err {
        cargo_metadata::Error::CargoMetadata { stderr } => anyhow!("{}", stderr.trim_end()),
        err => err.into(),
    })
}

pub(crate) fn cargo_metadata_no_deps_frozen(manifest_path: &Path) -> anyhow::Result<Metadata> {
    MetadataCommand::new()
        .manifest_path(manifest_path)
        .no_deps()
        .other_options(vec!["--frozen".to_owned()])
        .exec()
        .map_err(|err| match err {
            cargo_metadata::Error::CargoMetadata { stderr } => {
                anyhow!("{}", stderr.trim_start_matches("error: ").trim_end())
            }
            err => err.into(),
        })
}

pub(crate) fn read_package_metadata(
    manifest_path: &Path,
) -> anyhow::Result<PackageMetadataCargoAtcoder> {
    let toml = fs::read_to_string(manifest_path)
        .with_context(|| format!("Failed to read `{}`", manifest_path.display()))?;

    let manifest = toml::from_str::<CargoToml>(&toml)
        .with_context(|| format!("Failed to parse manifest at `{}`", manifest_path.display()))?;

    return Ok(manifest.package.metadata.cargo_atcoder);

    #[derive(Deserialize)]
    struct CargoToml {
        package: CargoTomlPackage,
    }

    #[derive(Deserialize, Default)]
    struct CargoTomlPackage {
        #[serde(default)]
        metadata: CargoTomlPackageMetadata,
    }

    #[derive(Deserialize, Default)]
    #[serde(rename_all = "kebab-case")]
    struct CargoTomlPackageMetadata {
        #[serde(default)]
        cargo_atcoder: PackageMetadataCargoAtcoder,
    }
}

#[derive(Deserialize, Default, Debug)]
pub(crate) struct PackageMetadataCargoAtcoder {
    problems: BTreeMap<String, PackageMetadataCargoAtcoderProblem>,
}

impl PackageMetadataCargoAtcoder {
    pub(crate) fn bin_name<'a>(&'a self, problem_id: &'a str) -> &'a str {
        self.problems
            .get(problem_id)
            .map(|PackageMetadataCargoAtcoderProblem { bin }| &**bin)
            .unwrap_or(problem_id)
    }
}

#[derive(Deserialize, Debug)]
struct PackageMetadataCargoAtcoderProblem {
    bin: String,
}

pub(crate) trait MetadataExt {
    fn all_members(&self) -> Vec<&Package>;
    fn query_for_member<'a>(&'a self, spec: Option<&str>) -> anyhow::Result<&'a Package>;
}

impl MetadataExt for Metadata {
    fn all_members(&self) -> Vec<&Package> {
        all_members(self).collect()
    }

    fn query_for_member<'a>(&'a self, spec: Option<&str>) -> anyhow::Result<&'a Package> {
        let cargo_exe = env::var_os("CARGO").with_context(|| "`$CARGO` should be present")?;
        let manifest_path = self
            .resolve
            .as_ref()
            .and_then(|Resolve { root, .. }| root.as_ref())
            .map(|id| self[id].manifest_path.clone())
            .unwrap_or_else(|| self.workspace_root.join("Cargo.toml"));
        let output = Command::new(cargo_exe)
            .arg("pkgid")
            .arg("--manifest-path")
            .arg(manifest_path)
            .args(spec)
            .output()?;
        let stdout = str::from_utf8(&output.stdout)?.trim_end();
        let stderr = str::from_utf8(&output.stderr)?.trim_end();
        if !output.status.success() {
            bail!("{}", stderr.trim_start_matches("error: "));
        }

        let url = stdout.parse::<Url>()?;
        let fragment = url.fragment().expect("the URL should contain fragment");
        let name = match *fragment.splitn(2, ':').collect::<Vec<_>>() {
            [name, _] => name,
            [_] => url
                .path_segments()
                .and_then(Iterator::last)
                .expect("should contain name"),
            _ => unreachable!(),
        };

        all_members(self).find(|p| p.name == name).with_context(|| {
            let spec = spec.expect("should be present here");
            format!("`{}` is not a member of the workspace", spec)
        })
    }
}

pub(crate) trait PackageExt {
    fn all_bins(&self) -> Vec<&Target>;
    fn find_bin<'a>(&'a self, name: &str) -> anyhow::Result<&'a Target>;
}

impl PackageExt for Package {
    fn all_bins(&self) -> Vec<&Target> {
        all_bins(self).collect()
    }

    fn find_bin<'a>(&'a self, name: &str) -> anyhow::Result<&'a Target> {
        all_bins(self)
            .find(|t| t.name == name)
            .with_context(|| format!("no bin target named `{}`", name))
    }
}

fn all_members(metadata: &Metadata) -> impl Iterator<Item = &Package> {
    metadata
        .packages
        .iter()
        .filter(move |Package { id, .. }| metadata.workspace_members.contains(id))
}

fn all_bins(package: &Package) -> impl Iterator<Item = &Target> {
    package
        .targets
        .iter()
        .filter(|Target { kind, .. }| kind.contains(&"bin".to_owned()))
}
