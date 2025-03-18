mod amalgamation;
mod gem;
mod gh_releases;
mod installer_sh;
mod manifest;
mod npm;
mod pip;
mod spec;
mod spm;
mod sqlpkg;
mod publish;

use clap::{builder::OsStr, value_parser, Arg, ArgMatches, Command};
use flate2::write::GzEncoder;
use flate2::Compression;
use manifest::write_manifest;
use npm::NpmBuildError;
use pip::PipBuildError;
use semver::Version;
use serde::{Serialize, Serializer};
use sha2::{Digest, Sha256};
use spec::Spec;
use std::{
    fs::{self, File},
    io::{self, Write},
    path::PathBuf,
};
use tar::Header;
use crate::publish::publish;

struct Project {
    version: Version,
    spec: Spec,
    spec_directory: PathBuf,
    platform_directories: Vec<PlatformDirectory>,
}

impl Project {
    pub(crate) fn release_download_url(&self, name: &str) -> String {
        let gh_base = self.spec.package.repo.clone();
        format!(
            "{gh_base}/releases/download/{}/{name}",
            self.spec.package.git_tag(&self.version)
        )
    }
}

#[derive(Debug, Clone)]
struct PlatformDirectory {
    os: Os,
    cpu: Cpu,
    _path: PathBuf,
    loadable_files: Vec<LoadablePlatformFile>,
    static_files: Vec<PlatformFile>,
    header_files: Vec<PlatformFile>,
}

#[derive(Debug, Clone)]
enum Os {
    Macos,
    Linux,
    Windows,
    Android,
    Ios,
    IosSimulator,
}

impl Serialize for Os {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.to_string().as_str())
    }
}

impl ToString for Os {
    fn to_string(&self) -> String {
        match self {
            Os::Macos => "macos".to_owned(),
            Os::Linux => "linux".to_owned(),
            Os::Windows => "windows".to_owned(),
            Os::Android => "android".to_owned(),
            Os::Ios => "ios".to_owned(),
            Os::IosSimulator => "iossimulator".to_owned(),
        }
    }
}

#[derive(Debug, Clone)]
enum Cpu {
    X86_64,
    Aarch64,
    I686,
    Armv7a,
}

impl Serialize for Cpu {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.to_string().as_str())
    }
}

impl ToString for Cpu {
    fn to_string(&self) -> String {
        match self {
            Cpu::X86_64 => "x86_64".to_owned(),
            Cpu::Aarch64 => "aarch64".to_owned(),
            Cpu::I686 => "i686".to_owned(),
            Cpu::Armv7a => "armv7a".to_owned(),
        }
    }
}

#[derive(Debug, Clone)]
struct GithubRelease {
    url: String,
    platform: (Os, Cpu),
}

#[derive(Debug, Clone)]
enum AssetPipWheel {
    Standard((Os, Cpu)),
    Pyodide,
}

#[derive(Debug, Clone)]
enum GeneratedAssetKind {
    Npm(Option<(Os, Cpu)>),
    Gem((Os, Cpu)),
    Pip(AssetPipWheel),
    Datasette,
    SqliteUtils,
    GithubReleaseLoadable(GithubRelease),
    GithubReleaseStatic(GithubRelease),
    Sqlpkg,
    Spm,
    Amalgamation,
    Manifest,
}

impl ToString for GeneratedAssetKind {
    fn to_string(&self) -> String {
        match self {
            GeneratedAssetKind::Npm(_) => "npm".to_owned(),
            GeneratedAssetKind::Gem(_) => "gem".to_owned(),
            GeneratedAssetKind::Pip(_) => "pip".to_owned(),
            GeneratedAssetKind::Datasette => "datasette".to_owned(),
            GeneratedAssetKind::SqliteUtils => "sqlite-utils".to_owned(),
            GeneratedAssetKind::GithubReleaseLoadable(_) => "github-release-loadable".to_owned(),
            GeneratedAssetKind::GithubReleaseStatic(_) => "github-release-static".to_owned(),
            GeneratedAssetKind::Sqlpkg => "sqlpkg".to_owned(),
            GeneratedAssetKind::Spm => "spm".to_owned(),
            GeneratedAssetKind::Amalgamation => "amalgamation".to_owned(),
            GeneratedAssetKind::Manifest => "sqlite-dist-manifest".to_owned(),
        }
    }
}
impl Serialize for GeneratedAssetKind {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.to_string().as_str())
    }
}

#[derive(Serialize)]
struct GeneratedAsset {
    kind: GeneratedAssetKind,
    name: String,
    path: String,
    checksum_sha256: String,
    size: usize,
}
impl GeneratedAsset {
    fn from(kind: GeneratedAssetKind, path: &PathBuf, contents: &[u8]) -> io::Result<Self> {
        File::create(path)?.write_all(contents)?;
        Ok(Self {
            kind,
            name: path.file_name().unwrap().to_str().unwrap().to_string(),
            path: path.to_str().unwrap().to_string(),
            checksum_sha256: base16ct::lower::encode_string(&Sha256::digest(contents)),
            size: contents.len(),
        })
    }
}
//{"kind": "github_release", "name": "...", "path": "./", "checksum_sha256": ""},

#[derive(Debug, Clone)]
struct PlatformFile {
    name: String,
    data: Vec<u8>,
    metadata: Option<std::fs::Metadata>,
}

#[derive(Debug, Clone)]
struct LoadablePlatformFile {
    file_stem: String,
    file: PlatformFile,
}

impl PlatformFile {
    fn new<S: Into<String>, D: Into<Vec<u8>>>(
        name: S,
        data: D,
        metadata: Option<fs::Metadata>,
    ) -> Self {
        Self {
            name: name.into(),
            data: data.into(),
            metadata,
        }
    }
}

use thiserror::Error;

fn create_targz(files: &[&PlatformFile]) -> io::Result<Vec<u8>> {
    let mut tar_gz = Vec::new();
    {
        let enc = GzEncoder::new(&mut tar_gz, Compression::default());
        let mut tar = tar::Builder::new(enc);
        for file in files {
            let mut header = Header::new_gnu();
            header.set_path(file.name.clone())?;
            header.set_size(file.data.len() as u64);
            // TODO: workaround for determinstic builds?
            header.set_mtime(0); 
            if let Some(metadata) = &file.metadata {
                header.set_metadata(metadata);
            } else {
                header.set_mode(0o700);
                header.set_mtime(
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_secs(),
                );
            }
            header.set_cksum();
            tar.append::<&[u8]>(&header, file.data.as_ref())?;
        }
        tar.finish()?;
    };
    Ok(tar_gz)
}

#[derive(Error, Debug)]
pub enum PlatformDirectoryError {
    #[error("I/O error: {0}")]
    IOError(#[from] io::Error),

    #[error("Expected name of directory")]
    MissingDirectoryName,
    #[error("directory or file name must contains only valid UTF-8 characters")]
    InvalidCharacters,
    #[error("directory {0} is not a valid platform directory. The format must be $OS-$CPU.")]
    InvalidDirectoryName(String),
    #[error("Invalid operating system '{0}'. Must be one of 'macos', 'linux', or 'windows'")]
    InvalidOsValue(String),
    #[error("Invalid CPU name '{0}'. Must be one of 'x86_64' or 'aarch64'")]
    InvalidCpuValue(String),
}

impl PlatformDirectory {
    fn from_path(base_path: PathBuf) -> Result<Self, PlatformDirectoryError> {
        let mut loadable_files = vec![];
        let mut static_files = vec![];
        let mut header_files = vec![];

        let dirname = base_path
            .components()
            .last()
            .ok_or(PlatformDirectoryError::MissingDirectoryName)?
            .as_os_str()
            .to_str()
            .ok_or(PlatformDirectoryError::InvalidCharacters)?;
        let mut s = dirname.split('-');
        let os = match s
            .next()
            .ok_or_else(|| PlatformDirectoryError::InvalidDirectoryName(dirname.to_owned()))?
        {
            "macos" => Os::Macos,
            "linux" => Os::Linux,
            "windows" => Os::Windows,
            "android" => Os::Android,
            "ios" => Os::Ios,
            "iossimulator" => Os::IosSimulator,
            os => return Err(PlatformDirectoryError::InvalidOsValue(os.to_owned())),
        };
        let cpu = match s
            .next()
            .ok_or_else(|| PlatformDirectoryError::InvalidDirectoryName(dirname.to_owned()))?
        {
            "x86_64" => Cpu::X86_64,
            "aarch64" => Cpu::Aarch64,
            "i686" => Cpu::I686,
            "armv7a" => Cpu::Armv7a,
            cpu => return Err(PlatformDirectoryError::InvalidCpuValue(cpu.to_owned())),
        };
        if s.next().is_some() {
            return Err(PlatformDirectoryError::InvalidDirectoryName(
                dirname.to_owned(),
            ));
        }

        let dir = fs::read_dir(&base_path)?;
        for entry in dir {
            let entry_path = entry?.path();
            match entry_path.extension().and_then(|e| e.to_str()) {
                Some("so") | Some("dll") | Some("dylib") => {
                    let name = entry_path
                        .file_name()
                        .expect("file_name to exist because there is an extension")
                        .to_str()
                        .ok_or(PlatformDirectoryError::InvalidCharacters)?
                        .to_string();
                    let data = fs::read(&entry_path)?;
                    let metadata = Some(fs::metadata(&entry_path)?);
                    let file_stem = entry_path
                        .file_stem()
                        .expect("file_stem to exist because there is an extension")
                        .to_str()
                        .ok_or(PlatformDirectoryError::InvalidCharacters)?
                        .to_string();
                    loadable_files.push(LoadablePlatformFile {
                        file_stem,
                        file: PlatformFile {
                            name: name.to_string(),
                            data,
                            metadata,
                        },
                    });
                }
                Some("a") => {
                    let name = entry_path
                        .file_name()
                        .expect("file_name to exist because there is an extension")
                        .to_str()
                        .ok_or(PlatformDirectoryError::InvalidCharacters)?
                        .to_string();
                    let data = fs::read(&entry_path)?;
                    let metadata = Some(fs::metadata(&entry_path)?);
                    static_files.push(PlatformFile {
                        name: name.to_string(),
                        data,
                        metadata,
                    });
                }
                Some("h") => {
                    let name = entry_path
                        .file_name()
                        .expect("file_name to exist because there is an extension")
                        .to_str()
                        .ok_or(PlatformDirectoryError::InvalidCharacters)?
                        .to_string();
                    let data = fs::read(&entry_path)?;
                    let metadata = Some(fs::metadata(&entry_path)?);
                    header_files.push(PlatformFile {
                        name: name.to_string(),
                        data,
                        metadata,
                    });
                }
                _ => {
                    println!("Warning: unknown file type in platform directory");
                }
            }
        }
        Ok(PlatformDirectory {
            os,
            cpu,
            _path: base_path,
            loadable_files,
            static_files,
            header_files,
        })
    }
}

#[derive(Error, Debug)]
pub enum BuildError {
    #[error("`{0}` is a required argument")]
    RequiredArg(String),

    #[error("`{0}` is a required argument")]
    InvalidSpec(toml::de::Error),

    #[error("specfile error: `{0}`")]
    SpecError(String),

    #[error("I/O error: {0}")]
    IoError(#[from] io::Error),

    #[error("Invalid platform directory: {0}")]
    PlayformDirectoryError(#[from] PlatformDirectoryError),

    #[error("Error building a pip package: {0}")]
    PipBuildEror(#[from] PipBuildError),

    #[error("Error building an npm package: {0}")]
    NpmBuildEror(#[from] NpmBuildError),
}

struct BuildArgs {
    input_directory: PathBuf,
    output_directory: PathBuf,
    config_path: PathBuf,
    version: Version,
}

fn build_args(matches: ArgMatches) -> Result<BuildArgs, BuildError> {
    let input_directory = matches
        .get_one::<PathBuf>("input")
        .ok_or_else(|| BuildError::RequiredArg("input".to_owned()))?;
    let output_directory = matches
        .get_one::<PathBuf>("output")
        .ok_or_else(|| BuildError::RequiredArg("output".to_owned()))?;
    let config_path = matches
        .get_one::<PathBuf>("file")
        .ok_or_else(|| BuildError::RequiredArg("file".to_owned()))?;
    let version = matches
        .get_one::<String>("version")
        .ok_or_else(|| BuildError::RequiredArg("version".to_owned()))?;
    let version = Version::parse(version).unwrap();
    Ok(BuildArgs {
        input_directory: input_directory.to_owned(),
        output_directory: output_directory.to_owned(),
        config_path: config_path.to_owned(),
        version,
    })
}
fn build(args: BuildArgs) -> Result<(), BuildError> {
    // Get the values of arguments

    std::fs::create_dir_all(args.output_directory.clone())?;

    let spec: Spec = match toml::from_str(fs::read_to_string(args.config_path.clone())?.as_str()) {
        Ok(spec) => spec,
        Err(err) => {
            eprintln!("{}", err);
            return Err(BuildError::InvalidSpec(err));
        }
    };

    if spec.targets.sqlpkg.is_some() && spec.targets.github_releases.is_none() {
        return Err(BuildError::SpecError(
            "sqlpkg target requires the github_releases target".to_owned(),
        ));
    }
    if spec.targets.spm.is_some() && spec.targets.github_releases.is_none() {
        return Err(BuildError::SpecError(
            "spm target requires the github_releases target".to_owned(),
        ));
    }
    if spec.targets.datasette.is_some() && spec.targets.pip.is_none() {
        return Err(BuildError::SpecError(
            "datasette target requires the pip target".to_owned(),
        ));
    }
    if spec.targets.sqlite_utils.is_some() && spec.targets.pip.is_none() {
        return Err(BuildError::SpecError(
            "sqlite_utils target requires the pip target".to_owned(),
        ));
    }

    let mut entries = fs::read_dir(args.input_directory)?
        .map(|entry| {
            Ok(entry
                .map_err(|_| {
                    BuildError::SpecError("Could not read entry in input directory".to_owned())
                })?
                .path())
        })
        .collect::<Result<Vec<PathBuf>, BuildError>>()?;

    let emscripten_dir = entries
        .iter()
        .position(|entry| entry.file_name() == Some(&OsStr::from("wasm32-emscripten")))
        .map(|item| entries.remove(item));

    let pyodide_dir = entries
        .iter()
        .position(|entry| entry.file_name() == Some(&OsStr::from("pyodide")))
        .map(|item| entries.remove(item));

    let platform_directories: Result<Vec<PlatformDirectory>, BuildError> = entries
        .iter()
        .map(|entry| {
            PlatformDirectory::from_path(entry.to_owned())
                .map_err(BuildError::PlayformDirectoryError)
        })
        .collect();
    let platform_directories = platform_directories?;

    let project = Project {
        version: args.version,
        spec,
        spec_directory: args
            .config_path
            .clone()
            .parent()
            .unwrap()
            .to_path_buf()
            .clone(),
        platform_directories,
    };

    let mut generated_assets: Vec<GeneratedAsset> = vec![];
    if project.spec.targets.github_releases.is_some() {
        let path = args.output_directory.join("github_releases");
        std::fs::create_dir(&path)?;
        let gh_release_assets = gh_releases::write_platform_files(&project, &path)?;

        if project.spec.targets.sqlpkg.is_some() {
            let sqlpkg_dir = args.output_directory.join("sqlpkg");
            std::fs::create_dir(&sqlpkg_dir)?;
            generated_assets.extend(sqlpkg::write_sqlpkg(&project, &sqlpkg_dir)?);
        };

        if project.spec.targets.spm.is_some() {
            let path = args.output_directory.join("spm");
            std::fs::create_dir(&path)?;
            generated_assets.extend(spm::write_spm(&project.spec, &gh_release_assets, &path)?);
        };

        if let Some(amalgamation_config) = &project.spec.targets.amalgamation {
            let amalgamation_path = args.output_directory.join("amalgamation");
            std::fs::create_dir(&amalgamation_path)?;
            generated_assets.extend(amalgamation::write_amalgamation(
                &project,
                &amalgamation_path,
                amalgamation_config,
            )?);
        };

        generated_assets.extend(gh_release_assets);
    };

    if project.spec.targets.pip.is_some() {
        let pip_path = args.output_directory.join("pip");
        std::fs::create_dir(&pip_path)?;
        generated_assets.extend(pip::write_base_packages(&project, &pip_path)?);
        if project.spec.targets.datasette.is_some() {
            let datasette_path = args.output_directory.join("datasette");
            std::fs::create_dir(&datasette_path)?;
            generated_assets.push(pip::write_datasette(&project, &datasette_path)?);
        }
        if project.spec.targets.sqlite_utils.is_some() {
            let sqlite_utils_path = args.output_directory.join("sqlite_utils");
            std::fs::create_dir(&sqlite_utils_path)?;
            generated_assets.push(pip::write_sqlite_utils(&project, &sqlite_utils_path)?);
        }
    };
    if project.spec.targets.npm.is_some() {
        let npm_output_directory = args.output_directory.join("npm");
        std::fs::create_dir(&npm_output_directory)?;
        generated_assets.extend(npm::write_npm_packages(
            &project,
            &npm_output_directory,
            &emscripten_dir,
        )?);
    };
    if let Some(gem_config) = &project.spec.targets.gem {
        let gem_path = args.output_directory.join("gem");
        std::fs::create_dir(&gem_path)?;
        generated_assets.extend(gem::write_gems(&project, &gem_path, gem_config)?);
    };

    let github_releases_checksums_txt = generated_assets
        .iter()
        .filter(|ga| {
            matches!(
                ga.kind,
                GeneratedAssetKind::GithubReleaseLoadable(_)
                    | GeneratedAssetKind::GithubReleaseStatic(_)
                    | GeneratedAssetKind::Sqlpkg
                    | GeneratedAssetKind::Spm
            )
        })
        .map(|ga| format!("{} {}", ga.name, ga.checksum_sha256))
        .collect::<Vec<String>>()
        .join("\n");
    File::create(args.output_directory.join("checksums.txt"))?
        .write_all(github_releases_checksums_txt.as_bytes())?;
    File::create(args.output_directory.join("install.sh"))?.write_all(
        crate::installer_sh::templates::install_sh(&project, &generated_assets).as_bytes(),
    )?;
    write_manifest(&args.output_directory, &generated_assets)?;

    publish(&generated_assets).unwrap();
    Ok(())
}

fn main() {
    let matches = Command::new(env!("CARGO_PKG_NAME"))
        .version(env!("CARGO_PKG_VERSION"))
        .author("Alex Garcia")
        .about("Package and distribute pre-compiled SQLite extensions")
        .arg(
            Arg::new("input")
                .long("input")
                .value_name("INPUT_DIR")
                .help("Sets the input directory")
                .value_parser(value_parser!(PathBuf)),
        )
        .arg(
            Arg::new("output")
                .long("output")
                .value_name("OUTPUT_DIR")
                .help("Sets the output directory")
                .value_parser(value_parser!(PathBuf)),
        )
        .arg(
            Arg::new("version")
                .long("version")
                .value_name("VERSION")
                .help("Set the version ")
                .required(true),
        )
        .arg(
            Arg::new("file")
                .value_name("FILE")
                .help("Sets the input file")
                .required(true)
                .index(1)
                .value_parser(value_parser!(PathBuf)),
        )
        .disable_version_flag(true)
        .get_matches();

    match build(build_args(matches).unwrap()) {
        Ok(_) => std::process::exit(0),
        Err(error) => {
            eprintln!("Build error: {error}");
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use flate2::read::GzDecoder;
    use std::{fs, io, path::Path};
    use tar::Archive;
    use tempdir::TempDir;

    use crate::*;

    fn walk_directory(root: &Path) -> Result<Vec<String>, io::Error> {
        let mut files = Vec::new();

        for entry in fs::read_dir(root)? {
            let entry = entry?;
            let path = entry.path();

            if path.is_file() {
                let filename = path.file_name().unwrap().to_string_lossy().into_owned();
                files.push(format!("{filename} {}", entry.metadata().unwrap().len()));
            } else if path.is_dir() {
                files.extend(walk_directory(&path)?);
            }
        }

        files.sort();
        Ok(files)
    }

    fn walk_directory2(root: &Path) -> Result<Vec<String>, io::Error> {
        let mut files = Vec::new();

        walk_directory_recursive(root, root, &mut files)?;

        files.sort();

        Ok(files)
    }

    fn walk_directory_recursive(
        root: &Path,
        current: &Path,
        files: &mut Vec<String>,
    ) -> Result<(), io::Error> {
        /*if root == current {
            return Ok(());
        }*/

        let rel_path = current.strip_prefix(root).unwrap();

        if let Some(size) = fs::metadata(current).map(|meta| meta.len()).ok() {
            files.push(format!("{}", rel_path.display()));
        }

        for entry in fs::read_dir(current)? {
            let entry = entry?;
            let path = entry.path();

            if path.is_file() {
                if let Some(size) = fs::metadata(&path).map(|meta| meta.len()).ok() {
                    // TODO file size but NPM packages are non-deterministic?
                    files.push(format!(
                        "{}",
                        path.strip_prefix(root).unwrap().display()
                    ));
                }
            } else if path.is_dir() {
                walk_directory_recursive(root, &path, files)?;
            }
        }

        Ok(())
    }

    #[test]
    fn foo() {
        let tmpdir = TempDir::new("sqlite-dist-test").unwrap();
        build(BuildArgs {
            input_directory: "sample/dist".into(),
            output_directory: tmpdir.path().join("out").into(),
            config_path: "sample/sqlite-dist.toml".into(),
            version: Version::new(0, 0, 1),
        })
        .unwrap();
        eprintln!("asdf {:?}", tmpdir);
        let mut ls: Vec<String> = fs::read_dir(tmpdir.path().join("out"))
            .unwrap()
            .map(|p| {
                let entry = p.unwrap();
                let file_type = entry.file_type().unwrap();
                let metadata = entry.metadata().unwrap();
                let filename = entry.file_name().to_string_lossy().to_string();

                format!("{} {}", filename, metadata.len())
            })
            .collect();
        ls.sort();
        insta::assert_yaml_snapshot!(walk_directory2(&tmpdir.path().join("out")).unwrap());
        Archive::new(GzDecoder::new(
            File::open(tmpdir
                .path()
                .join("out")
                .join("npm")
                .join("sqlite-sample.tar.gz")).unwrap(),
        ));
        /*
        let x = fs::read_to_string(tmpdir.path().join("out").join("sqlite-dist-manifest.json"))
            .unwrap();
        let x = x.replace(tmpdir.path().to_str().unwrap(), "REDACTED");
        insta::assert_snapshot!(x);
         */

    }
}
