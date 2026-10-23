//! 이 머신에 설정된 AWS 프로필 목록(설정 창의 Bedrock 프로필 고르기용).
//!
//! AWS CLI와 같은 파일을 본다: `AWS_CONFIG_FILE`(기본 `~/.aws/config`)과
//! `AWS_SHARED_CREDENTIALS_FILE`(기본 `~/.aws/credentials`). 자격 증명 파일에서는 섹션 이름만
//! 읽고 키 값은 읽지 않는다.

use std::path::PathBuf;

/// 설정 창에서 고를 수 있는 Bedrock 리전(코드, 이름). 상용 리전만 둔다.
pub const BEDROCK_REGIONS: &[(&str, RegionName)] = &[
    ("us-east-1", RegionName("us-east-1")),
    ("us-east-2", RegionName("us-east-2")),
    ("us-west-2", RegionName("us-west-2")),
    ("ca-central-1", RegionName("ca-central-1")),
    ("sa-east-1", RegionName("sa-east-1")),
    ("eu-central-1", RegionName("eu-central-1")),
    ("eu-central-2", RegionName("eu-central-2")),
    ("eu-west-1", RegionName("eu-west-1")),
    ("eu-west-2", RegionName("eu-west-2")),
    ("eu-west-3", RegionName("eu-west-3")),
    ("eu-north-1", RegionName("eu-north-1")),
    ("eu-south-1", RegionName("eu-south-1")),
    ("eu-south-2", RegionName("eu-south-2")),
    ("ap-northeast-1", RegionName("ap-northeast-1")),
    ("ap-northeast-2", RegionName("ap-northeast-2")),
    ("ap-northeast-3", RegionName("ap-northeast-3")),
    ("ap-south-1", RegionName("ap-south-1")),
    ("ap-south-2", RegionName("ap-south-2")),
    ("ap-southeast-1", RegionName("ap-southeast-1")),
    ("ap-southeast-2", RegionName("ap-southeast-2")),
];

/// 리전 이름. 보일 때(`Display`) UI 언어로 옮긴다(ADR-0027, `assistant.region.<코드>`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RegionName(&'static str);

impl std::fmt::Display for RegionName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&t!(format!("assistant.region.{}", self.0)))
    }
}

/// 프로필 하나.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AwsProfile {
    pub name: String,
    /// config 파일에 적힌 리전. 없으면 SDK 기본(환경 변수 등)을 따른다.
    pub region: Option<String>,
}

/// 설정된 프로필. `default`가 맨 앞이고 나머지는 이름순이다. 파일이 없으면 빈 목록.
pub fn list() -> Vec<AwsProfile> {
    let read = |path: Option<PathBuf>| {
        path.and_then(|path| std::fs::read_to_string(path).ok())
            .unwrap_or_default()
    };
    parse(
        &read(file_path("AWS_CONFIG_FILE", "config")),
        &read(file_path("AWS_SHARED_CREDENTIALS_FILE", "credentials")),
    )
}

/// 환경 변수로 바꾼 경로, 없으면 `~/.aws/<name>`.
fn file_path(env: &str, name: &str) -> Option<PathBuf> {
    std::env::var_os(env)
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|home| home.join(".aws").join(name)))
}

/// 섹션 머리 줄(`[이름]`)이면 안쪽 이름.
fn section(line: &str) -> Option<&str> {
    line.strip_prefix('[')?.strip_suffix(']').map(str::trim)
}

/// 이름이 같은 프로필의 위치. 없으면 새로 더한다.
fn add(profiles: &mut Vec<AwsProfile>, name: &str) -> usize {
    if let Some(ix) = profiles.iter().position(|profile| profile.name == name) {
        return ix;
    }
    profiles.push(AwsProfile {
        name: name.to_string(),
        region: None,
    });
    profiles.len() - 1
}

/// config·credentials 파일 내용에서 프로필을 모은다.
fn parse(config: &str, credentials: &str) -> Vec<AwsProfile> {
    let mut profiles: Vec<AwsProfile> = Vec::new();
    // config: `[default]`, `[profile 이름]`. `[sso-session …]`·`[services …]`는 프로필이 아니다.
    let mut current = None;
    for line in config.lines().map(str::trim) {
        if line.starts_with(['#', ';']) {
            continue;
        }
        if let Some(name) = section(line) {
            current = match name {
                "default" => Some(add(&mut profiles, "default")),
                _ => name
                    .strip_prefix("profile ")
                    .map(str::trim)
                    .filter(|name| !name.is_empty())
                    .map(|name| add(&mut profiles, name)),
            };
        } else if let Some(ix) = current
            && let Some((key, value)) = line.split_once('=')
            && key.trim() == "region"
        {
            let value = value.trim();
            if !value.is_empty() {
                profiles[ix].region = Some(value.to_string());
            }
        }
    }
    // credentials: `[이름]` 섹션만 본다.
    for line in credentials.lines().map(str::trim) {
        if let Some(name) = section(line).filter(|name| !name.is_empty()) {
            add(&mut profiles, name);
        }
    }

    profiles.sort_by(|a, b| {
        (a.name != "default")
            .cmp(&(b.name != "default"))
            .then_with(|| a.name.cmp(&b.name))
    });
    profiles
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collects_profiles_from_both_files() {
        let config = "\
# 주석
[default]
region = us-east-1

[profile writer-dev]
region=ap-northeast-2
output = json

[sso-session corp]
sso_region = us-west-2

[profile beta]
";
        let credentials = "\
[default]
aws_access_key_id = AKIA...
[legacy]
aws_secret_access_key = ...
";
        let profiles = parse(config, credentials);
        let names: Vec<_> = profiles.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["default", "beta", "legacy", "writer-dev"]);
        assert_eq!(profiles[0].region.as_deref(), Some("us-east-1"));
        assert_eq!(profiles[1].region, None);
        assert_eq!(profiles[3].region.as_deref(), Some("ap-northeast-2"));
    }

    #[test]
    fn empty_files_have_no_profiles() {
        assert!(parse("", "").is_empty());
        assert!(parse("[profile ]\nregion = x\n", "").is_empty());
    }
}
