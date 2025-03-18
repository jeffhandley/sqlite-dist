use std::fs::File;
use std::io::Read;
use std::path::Path;
use sha2::{Sha256, Digest};

use crate::{GeneratedAsset, GeneratedAssetKind};

const BASE_URL_GITHUB_RELEASE_ASSET: &str = "https://uploads.github.com";
const BASE_URL_RUBYGEMS: &str = "https://rubygems.org/api/v1/gems";
const BASE_URL_PYPI: &str = "https://upload.pypi.org/legacy/";


/// https://docs.github.com/en/rest/releases/assets?apiVersion=2022-11-28#upload-a-release-asset
pub fn upload_github_asset(
  owner: &str,
  repo: &str,
  release_id: &str,
  name: &str,
  file_path: &str,
  token: &str,
  dry_run: bool,
) -> Result<(), Box<dyn std::error::Error>> {
  let file_name = Path::new(file_path).file_name().unwrap().to_str().unwrap();
  let url = format!(
      "{}/repos/{}/{}/releases/{}/assets?name={}",
      BASE_URL_GITHUB_RELEASE_ASSET, owner, repo, release_id, file_name
  );
  
  let mut file = File::open(file_path)?;
  let mut data = Vec::new();
  file.read_to_end(&mut data)?;
  
  let client = reqwest::blocking::Client::new();
  let request = client.post(&url)
      .header("Accept", "application/vnd.github+json")
      .header("Authorization", format!("Bearer {}", token))
      .header("X-GitHub-Api-Version", "2022-11-28")
      .header("Content-Type", "application/octet-stream")
      .body(data);

  if dry_run {
    let request = request.build().unwrap();
    println!("{}", "=".repeat(40));
    println!("GHA dry run");
    println!("{}", request.url());
    for h in request.headers() {
      println!("{}: {}", h.0, h.1.to_str().unwrap());
    }
    println!("Request body size: {}", request.body().unwrap().as_bytes().unwrap().len());
    println!("{}", "=".repeat(40));
    Ok(())
  }else {
    let response = request.send()?;
  
    if response.status().is_success() {
        println!("File uploaded successfully");
        Ok(())
    } else {
      let status = response.status();
        let error_text = response.text()?.clone();
        eprintln!("Upload failed: {}", error_text);
        Err(format!("Upload failed with status {}: {}", status, error_text).into())
    }
  }
  
}

pub fn upload_gem(
  gem_path: &str,
  api_key: &str,
  dry_run: bool,
) -> Result<(), Box<dyn std::error::Error>> {
  let mut file = File::open(gem_path)?;
  let mut data = Vec::new();
  file.read_to_end(&mut data)?;
  
  let client = reqwest::blocking::Client::new();
  let request = client.post(BASE_URL_RUBYGEMS)
      .header("Authorization", api_key)
      .header("Content-Type", "application/octet-stream")
      .body(data);
  if dry_run {
    let request = request.build().unwrap();
    println!("{}", "=".repeat(40));
    println!("Gem dry run ({})", gem_path);
    println!("{}", request.url());
    for h in request.headers() {
      println!("{}: {}", h.0, h.1.to_str().unwrap());
    }
    println!("Request body size: {}", request.body().unwrap().as_bytes().unwrap().len());
    println!("{}", "=".repeat(40));
    Ok(())
  }else {
    let response = request.send()?;
  
    if response.status().is_success() {
        println!("Gem uploaded successfully");
        Ok(())
    } else {
      let status = response.status();
        let error_text = response.text()?;
        eprintln!("Gem upload failed: {}", error_text);
        Err(format!("Upload failed with status {}: {}", status, error_text).into())
    }
  }
}

pub fn upload_pypi(
  file_path: &str,
  package_name: &str,
  package_version: &str,
  username: &str,
  password: &str,
  file_type: &str,  // "bdist_wheel" or "sdist"
  python_version: &str, // Python tag for wheels or "source" for sdist
  description: Option<&str>,
  dry_run: bool,
) -> Result<(), Box<dyn std::error::Error>> {
  let path = Path::new(file_path);
  let file_name = path.file_name()
      .ok_or("Invalid file path")?
      .to_str()
      .ok_or("Invalid file name")?;
  
  let data = std::fs::read(file_path)?;
  
  // Calculate SHA256 hash for the file
  let mut hasher = Sha256::new();
  hasher.update(&data);
  let sha256_digest = format!("{:x}", hasher.finalize());
  
  // Create multipart form
  let mut form = reqwest::blocking::multipart::Form::new()
      .text("name", package_name.to_string())
      .text("version", package_version.to_string())
      .text(":action", "file_upload")
      .text("protocol_version", "1")
      .text("filetype", file_type.to_string())
      .text("pyversion", python_version.to_string())
      .text("sha256_digest", sha256_digest)
      .text("metadata_version", "2.1");
  
  // Add optional description if provided
  if let Some(desc) = description {
      form = form.text("description", desc.to_string());
  }
  
  // Add file content
  let file_part = reqwest::blocking::multipart::Part::bytes(data)
      .file_name(file_name.to_string())
      .mime_str("application/octet-stream")?;
  
  form = form.part("content", file_part);
  
  // Send request with Basic Auth
  let client = reqwest::blocking::Client::new();
  let request = client.post(BASE_URL_PYPI)
      .basic_auth(username, Some(password))
      .multipart(form);

    if dry_run {
      let request = request.build().unwrap();
      println!("{}", "=".repeat(40)); 
      println!("PyPI dry run ({})", file_path);
      println!("{}", request.url());
      for h in request.headers() {
        println!("{}: {}", h.0, h.1.to_str().unwrap());
      }
      //println!("Request body size: {}", request.body().unwrap().as_bytes().unwrap().len());
      println!("{}", "=".repeat(40));
      Ok(())
    }else {
      let response = request.send()?;
  
  if response.status().is_success() {
      println!("PyPI package uploaded successfully");
      Ok(())
  } else {
      let status = response.status();
      let error_text = response.text()?;
      eprintln!("PyPI upload failed: {}", error_text);
      Err(format!("Upload failed with status {}: {}", status, error_text).into())
  }
    }
  
}

use sha2::{Sha512, Digest as Sha2Digest};
//use sha1::Digest as Sha1Digest;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use serde_json::{json, Value};

pub fn upload_npm(
  package_path: &str,
  package_name: &str,
  package_version: &str,
  registry_url: &str, 
  token: &str,
  tag: &str,
  access: Option<&str>,
  dry_run: bool,
) -> Result<(), Box<dyn std::error::Error>> {
  
  // Read the tarball file
  let tarball_bytes = std::fs::read(package_path)?;
  
  // Calculate the integrity hash (SHA512)
  let mut sha512 = Sha512::new();
  sha512.update(&tarball_bytes);
  let integrity = sha512.finalize();
  
  // Calculate the SHA1 hash
  let mut sha1_hasher = sha1::Sha1::new();
  sha1_hasher.update(&tarball_bytes);
  let shasum = sha1_hasher.finalize();
  
  // Format version without build tag if any
  let version_without_build_tag = match package_version.split('+').next() {
      Some(v) => v,
      None => package_version,
  };
  
  // Base URL for the registry
  let url = format!("{}/{}", 
      registry_url.trim_end_matches('/'),
      package_name.replace("@", "%40")
  );
  
  // Encode tarball data in base64
  let encoded_tarball = BASE64.encode(&tarball_bytes);
  
  // Generate the filename for the tarball
  let tarball_filename = format!("{}-{}.tgz", 
      package_name.split('/').last().unwrap_or(package_name), 
      package_version
  );
  
  // Calculate the integrity string and shasum string
  let integrity_str = format!("sha512-{}", BASE64.encode(&integrity));
  let shasum_str = format!("{:x}", shasum);
  
  // Create the JSON body for publishing using serde_json
  let dist_tags = json!({
      "latest": version_without_build_tag
  });

  let version_data = json!({}); // Empty JSON object for the version data

  let attachment = json!({
      "content_type": "application/octet-stream",
      "data": encoded_tarball,
      "length": tarball_bytes.len()
  });
  
  let mut attachments = serde_json::Map::new();
  attachments.insert(tarball_filename, attachment);

  // Construct the full JSON body
  let mut body_map = serde_json::Map::new();
  body_map.insert("_id".to_string(), Value::String(package_name.to_string()));
  body_map.insert("name".to_string(), Value::String(package_name.to_string()));
  body_map.insert("dist-tags".to_string(), dist_tags);
  
  // Add versions object
  let mut versions = serde_json::Map::new();
  versions.insert(version_without_build_tag.to_string(), version_data);
  body_map.insert("versions".to_string(), Value::Object(versions));
  
  // Add access (public or restricted)
  if let Some(acc) = access {
      body_map.insert("access".to_string(), Value::String(acc.to_string()));
  } else {
      body_map.insert("access".to_string(), Value::String("public".to_string()));
  }
  
  // Add attachments
  body_map.insert("_attachments".to_string(), Value::Object(attachments));
  
  let body_json = Value::Object(body_map);
  let body = serde_json::to_string(&body_json)?;
  
  // Create the client and send the request
  let client = reqwest::blocking::Client::new();
  let request = client.put(&url)
      .header("Content-Type", "application/json")
      .header("Authorization", format!("Bearer {}", token))
      .header("Accept", "*/*")
      .header("npm-command", "publish")
      .header("npm-auth-type", "legacy")
      .body(body);

  if dry_run {
    let request = request.build().unwrap();
    println!("{}", "=".repeat(40));
    println!("NPM dry run ({})", package_path);
    println!("{}", request.url());
    for h in request.headers() {
      println!("{}: {}", h.0, h.1.to_str().unwrap());
    }
    println!("{}", String::from_utf8(request.body().unwrap().as_bytes().unwrap().to_vec()).unwrap());
    println!("{}", "=".repeat(40));
    Ok(())
  }else {
    let response = request.send()?;
    if response.status().is_success() {
        println!("NPM package uploaded successfully");
        Ok(())
    } else {
        let status = response.status();
        let error_text = response.text()?;
        eprintln!("NPM upload failed: {}", error_text);
        Err(format!("Upload failed with status {}: {}", status, error_text).into())
    }
  }
  
}

pub fn publish(generated_assets: &Vec<GeneratedAsset>)->Result<(), String> {
  let gh_token = std::env::var("GH_TOKEN").unwrap();
  let npm_token = std::env::var("NPM_TOKEN").unwrap();
  let pypi_token = std::env::var("PYPI_API_TOKEN").unwrap();
  let gem_token = std::env::var("GEM_HOST_API_KEY").unwrap();

  let gh_releases = generated_assets
        .iter()
        .filter_map(|ga| match &ga.kind {
            GeneratedAssetKind::Manifest => Some(ga),
            GeneratedAssetKind::GithubReleaseLoadable(_)
            | GeneratedAssetKind::GithubReleaseStatic(_)
            | GeneratedAssetKind::Sqlpkg
            | GeneratedAssetKind::Spm
            | GeneratedAssetKind::Amalgamation => Some(ga),
            GeneratedAssetKind::Gem(_)
            | GeneratedAssetKind::Pip(_)
            | GeneratedAssetKind::Datasette
            | GeneratedAssetKind::SqliteUtils
            | GeneratedAssetKind::Npm(_) => None,
        })
        .collect::<Vec<&GeneratedAsset>>();
    let npm_releases = generated_assets
        .iter()
        .filter_map(|ga| match &ga.kind {
            GeneratedAssetKind::Npm(_) => Some(ga),
            _ => None,
        })
        .collect::<Vec<&GeneratedAsset>>();
    let pypi_releases = generated_assets
        .iter()
        .filter_map(|ga| match &ga.kind {
            GeneratedAssetKind::Pip(_) | GeneratedAssetKind::Datasette | GeneratedAssetKind::SqliteUtils => Some(ga),
            _ => None,
        })
        .collect::<Vec<&GeneratedAsset>>();
    let gem_releases = generated_assets
        .iter()
        .filter_map(|ga| match &ga.kind {
            GeneratedAssetKind::Gem(_) => Some(ga),
            _ => None,
        })
        .collect::<Vec<&GeneratedAsset>>();
    for gh_release in gh_releases {
        upload_github_asset("owner", "repo", "release_id", "name", &gh_release.path, &gh_token, true);
    }
    for gem_release in gem_releases {
      upload_gem(&gem_release.path, &gem_token, true);
  }
    for npm_release in npm_releases {
        upload_npm(&npm_release.path, "npm_release", "package_version", "https://registry.npmjs.org", &npm_token, "tag", None, true);
        
    }
    for pypi_release in pypi_releases {
        upload_pypi(&pypi_release.path, "package_name", "package_version", "__token__", &pypi_token, "file_type", "python_version", None, true);
    }
    
  Ok(())
}