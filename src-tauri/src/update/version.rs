use std::cmp::Ordering;

use crate::error::AppError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Version {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
    pub build: u64,
    pub nightly: Option<NightlyVersion>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NightlyVersion {
    pub date: u32,
    pub sequence: u32,
}

impl Version {
    pub fn parse(value: &str) -> Result<Self, AppError> {
        let (base, suffix) = value
            .split_once('-')
            .ok_or_else(|| AppError::InvalidData("version is missing its channel suffix".into()))?;
        let mut base_parts = base.split('.');
        let major = parse_number(base_parts.next(), "major")?;
        let minor = parse_number(base_parts.next(), "minor")?;
        let patch = parse_number(base_parts.next(), "patch")?;
        if base_parts.next().is_some() {
            return Err(AppError::InvalidData(
                "version base has too many components".into(),
            ));
        }
        let mut suffix_parts = suffix.split('.');
        if suffix_parts.next() != Some("tabbyrs") {
            return Err(AppError::InvalidData(
                "version has an unknown product suffix".into(),
            ));
        }
        let build = parse_number(suffix_parts.next(), "build")?;
        let nightly = match suffix_parts.next() {
            None => None,
            Some("nightly") => {
                let date = parse_date(suffix_parts.next())?;
                let sequence =
                    u32::try_from(parse_number(suffix_parts.next(), "nightly sequence")?).map_err(
                        |_| AppError::InvalidData("nightly sequence is too large".into()),
                    )?;
                if suffix_parts.next().is_some() {
                    return Err(AppError::InvalidData(
                        "nightly version has too many components".into(),
                    ));
                }
                Some(NightlyVersion { date, sequence })
            }
            Some(_) => {
                return Err(AppError::InvalidData(
                    "version has an unknown channel suffix".into(),
                ))
            }
        };
        Ok(Self {
            major,
            minor,
            patch,
            build,
            nightly,
        })
    }

    pub fn is_stable(&self) -> bool {
        self.nightly.is_none()
    }

    pub fn is_newer_than(&self, current: &Self) -> bool {
        if self.nightly.is_some() != current.nightly.is_some() {
            return false;
        }
        self.cmp(current) == Ordering::Greater
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> Ordering {
        (self.major, self.minor, self.patch, self.build)
            .cmp(&(other.major, other.minor, other.patch, other.build))
            .then_with(|| match (&self.nightly, &other.nightly) {
                (None, None) => Ordering::Equal,
                (Some(_), None) => Ordering::Less,
                (None, Some(_)) => Ordering::Greater,
                (Some(left), Some(right)) => {
                    (left.date, left.sequence).cmp(&(right.date, right.sequence))
                }
            })
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

fn parse_number(value: Option<&str>, label: &str) -> Result<u64, AppError> {
    let value =
        value.ok_or_else(|| AppError::InvalidData(format!("version is missing {label}")))?;
    if value.is_empty() || (value.len() > 1 && value.starts_with('0')) {
        return Err(AppError::InvalidData(format!("version {label} is invalid")));
    }
    value
        .parse::<u64>()
        .map_err(|_| AppError::InvalidData(format!("version {label} is invalid")))
}

fn parse_date(value: Option<&str>) -> Result<u32, AppError> {
    let value =
        value.ok_or_else(|| AppError::InvalidData("nightly version is missing its date".into()))?;
    if value.len() != 8 || !value.chars().all(|character| character.is_ascii_digit()) {
        return Err(AppError::InvalidData(
            "nightly version date is invalid".into(),
        ));
    }
    let date = value
        .parse::<u32>()
        .map_err(|_| AppError::InvalidData("nightly version date is invalid".into()))?;
    let month = (date / 100) % 100;
    let day = date % 100;
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return Err(AppError::InvalidData(
            "nightly version date is invalid".into(),
        ));
    }
    Ok(date)
}

#[cfg(test)]
mod tests {
    use super::Version;

    #[test]
    fn parses_stable_and_nightly_versions_without_semver_assumptions() {
        assert!(Version::parse("1.0.231-tabbyrs.4").unwrap().is_stable());
        let nightly = Version::parse("1.0.231-tabbyrs.4.nightly.20260812.2").unwrap();
        assert!(!nightly.is_stable());
        assert!(
            nightly.is_newer_than(&Version::parse("1.0.231-tabbyrs.3.nightly.20260811.9").unwrap())
        );
    }

    #[test]
    fn stable_and_nightly_are_not_interchangeable() {
        let stable = Version::parse("1.0.231-tabbyrs.4").unwrap();
        let nightly = Version::parse("1.0.231-tabbyrs.5.nightly.20260812.1").unwrap();
        assert!(!nightly.is_newer_than(&stable));
        assert!(!stable.is_newer_than(&nightly));
    }

    #[test]
    fn rejects_ambiguous_or_invalid_versions() {
        for value in [
            "1.0.231",
            "01.0.231-tabbyrs.1",
            "1.0.231-tabbyrs.1.preview",
            "1.0.231-tabbyrs.1.nightly.20260299.1",
        ] {
            assert!(Version::parse(value).is_err(), "{value}");
        }
    }
}
