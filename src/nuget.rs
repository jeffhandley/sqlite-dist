use crate::{Cpu, GeneratedAsset, GeneratedAssetKind, Os, Project};
use serde::{Deserialize, Serialize};
use std::{
    io::{self, Cursor, Write},
    path::Path,
};
use zip::{write::FileOptions, ZipWriter};

#[derive(Debug, Deserialize, Serialize)]
pub struct Nuspec {
    pub metadata: Metadata,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Metadata {
    pub id: String,
    pub version: String,
    pub title: String,
    pub authors: String,
    pub owners: String,
    pub require_license_acceptance: bool,
    pub description: String,
}

impl Nuspec {
    fn new(project: &Project) -> Self {
        let author = project.spec.package.authors.first().unwrap();
        Self {
            metadata: Metadata {
                id: project.spec.package.name.clone(),
                version: project.version.to_string(),
                title: project.spec.package.name.clone(),
                authors: author.clone(),
                owners: author.clone(),
                require_license_acceptance: false,
                description: project.spec.package.description.clone(),
            },
        }
    }

    fn to_xml(&self) -> String {
        format!(
            r#"<?xml version="1.0" encoding="utf-8"?>
<package xmlns="http://schemas.microsoft.com/packaging/2010/07/nuspec.xsd">
  <metadata>
    <id>{}</id>
    <version>{}</version>
    <title>{}</title>
    <authors>{}</authors>
    <owners>{}</owners>
    <requireLicenseAcceptance>{}</requireLicenseAcceptance>
    <description>{}</description>
  </metadata>
</package>"#,
            self.metadata.id,
            self.metadata.version,
            self.metadata.title,
            self.metadata.authors,
            self.metadata.owners,
            self.metadata.require_license_acceptance,
            self.metadata.description
        )
    }
}

pub struct NugetPackage {
    zip: ZipWriter<Cursor<Vec<u8>>>,
}

impl NugetPackage {
    pub fn new() -> io::Result<Self> {
        let buffer = Vec::new();
        let cursor = Cursor::new(buffer);
        let mut zip = ZipWriter::new(cursor);
        zip.start_file("[Content_Types].xml", FileOptions::default())?;
        zip.write_all(Self::content_types().as_bytes())?;
        Ok(Self { zip })
    }

    fn content_types() -> String {
        r#"<?xml version="1.0" encoding="utf-8"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
  <Default Extension="xml" ContentType="application/xml"/>
  <Default Extension="nuspec" ContentType="application/octet-stream"/>
  <Default Extension="psmdcp" ContentType="application/vnd.openxmlformats-package.core-properties+xml"/>
</Types>"#.to_string()
    }

    pub fn add_nuspec(&mut self, nuspec: &Nuspec) -> io::Result<()> {
        let nuspec_file_name = format!("{}.nuspec", nuspec.metadata.id);
        let nuspec_content = nuspec.to_xml();
        self.zip
            .start_file(&nuspec_file_name, FileOptions::default())?;
        self.zip.write_all(nuspec_content.as_bytes())?;
        Ok(())
    }

    pub fn add_files(&mut self, project: &Project) -> io::Result<()> {
        for platform_dir in &project.platform_directories {
            if !(matches!(platform_dir.os, Os::Linux | Os::Macos | Os::Windows)
                && matches!(platform_dir.cpu, Cpu::X86_64 | Cpu::Aarch64))
            {
                continue;
            }
            for loadable_file in &platform_dir.loadable_files {
                let os = match platform_dir.os {
                    Os::Windows => "win",
                    Os::Linux => "linux",
                    Os::Macos => "osx",
                    Os::Android => "android",
                    Os::Ios => "ios",
                    Os::IosSimulator => "ios-simulator",
                };
                let cpu = match platform_dir.cpu {
                    Cpu::Aarch64 => "arm64",
                    Cpu::X86_64 => "x64",
                    Cpu::I686 => "x86",
                    Cpu::Armv7a => "armv7",
                };
                let file_path = format!("runtimes/{}-{}/native/{}", os, cpu, loadable_file.file.name);
                self.zip.start_file(&file_path, FileOptions::default())?;
                self.zip.write_all(&loadable_file.file.data)?;
            }
        }
        Ok(())
    }

    pub fn finish(mut self) -> io::Result<Vec<u8>> {
        let result = self.zip.finish()?;
        Ok(result.into_inner())
    }
}

pub(crate) fn write_nuget_packages(
    project: &Project,
    nuget_output_directory: &Path,
) -> io::Result<Vec<GeneratedAsset>> {
    let mut assets = vec![];
    let nuspec = Nuspec::new(project);
    let mut package = NugetPackage::new()?;
    package.add_nuspec(&nuspec)?;
    package.add_files(project)?;
    let buffer = package.finish()?;
    let output_path = nuget_output_directory.join(format!(
        "{}.{}.nupkg",
        nuspec.metadata.id, nuspec.metadata.version
    ));
    std::fs::write(&output_path, &buffer)?;
    assets.push(GeneratedAsset::from(
        GeneratedAssetKind::Nuget,
        &output_path,
        &buffer,
    )?);
    Ok(assets)
}
