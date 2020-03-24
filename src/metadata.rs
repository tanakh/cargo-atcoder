use anyhow::{anyhow, ensure, Context as _};
use cargo_metadata::{Metadata, MetadataCommand, Package, Resolve, Target};
use std::{env, path::Path, process::Command, str};
use url::Url;

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

pub(crate) trait MetadataExt {
    fn all_members(&self) -> Vec<&Package>;
    fn query_for_member<'a>(&'a self, spec: Option<&str>) -> anyhow::Result<&'a Package>;
    fn find_bin<'a>(&'a self, bin_name: &str) -> anyhow::Result<(&'a Target, &'a Package)>;
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
        ensure!(output.status.success(), "{}", stderr);

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

    fn find_bin<'a>(&'a self, bin_name: &str) -> anyhow::Result<(&'a Target, &'a Package)> {
        all_members(self)
            .flat_map(|p| all_bins(p).map(move |t| (t, p)))
            .find(|(Target { name, .. }, _)| name == bin_name)
            .with_context(|| format!("no bin target named `{}`", bin_name))
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
