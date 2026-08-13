use std::cmp::Ordering;

use crate::error::AppError;
use crate::storage::state_file::UpdateChannel;

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

    pub fn is_newer_for_channel(&self, current: &Self, channel: &UpdateChannel) -> bool {
        match channel {
            UpdateChannel::Stable => match (&self.nightly, &current.nightly) {
                (None, None) => self.cmp(current) == Ordering::Greater,
                (None, Some(_)) => self.base_cmp(current) != Ordering::Less,
                _ => false,
            },
            UpdateChannel::Nightly => match (&self.nightly, &current.nightly) {
                (Some(_), Some(_)) => self.cmp(current) == Ordering::Greater,
                (Some(_), None) => self.base_cmp(current) != Ordering::Less,
                _ => false,
            },
        }
    }

    fn base_cmp(&self, other: &Self) -> Ordering {
        (self.major, self.minor, self.patch, self.build).cmp(&(
            other.major,
            other.minor,
            other.patch,
            other.build,
        ))
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
    let year = date / 10_000;
    let month = (date / 100) % 100;
    let day = date % 100;
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let max_day = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => 0,
    };
    if year == 0 || day == 0 || day > max_day {
        return Err(AppError::InvalidData(
            "nightly version date is invalid".into(),
        ));
    }
    Ok(date)
}

#[cfg(test)]
mod tests {
    use super::Version;
    use crate::storage::state_file::UpdateChannel;

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
        let nightly = Version::parse("1.0.231-tabbyrs.4.nightly.20260812.1").unwrap();
        assert!(!nightly.is_newer_than(&stable));
        assert!(!stable.is_newer_than(&nightly));
        assert!(nightly.is_newer_for_channel(&stable, &UpdateChannel::Nightly));
        assert!(stable.is_newer_for_channel(&nightly, &UpdateChannel::Stable));
    }

    #[test]
    fn channel_switch_requires_the_same_or_newer_upstream_build() {
        let stable = Version::parse("1.0.231-tabbyrs.4").unwrap();
        let nightly = Version::parse("1.0.231-tabbyrs.4.nightly.20260812.1").unwrap();
        let older_nightly = Version::parse("1.0.231-tabbyrs.3.nightly.20260812.1").unwrap();
        let older_stable = Version::parse("1.0.231-tabbyrs.3").unwrap();

        assert!(nightly.is_newer_for_channel(&stable, &UpdateChannel::Nightly));
        assert!(!older_nightly.is_newer_for_channel(&stable, &UpdateChannel::Nightly));
        assert!(stable.is_newer_for_channel(&nightly, &UpdateChannel::Stable));
        assert!(!older_stable.is_newer_for_channel(&nightly, &UpdateChannel::Stable));
    }

    #[test]
    fn rejects_ambiguous_or_invalid_versions() {
        for value in [
            "1.0.231",
            "01.0.231-tabbyrs.1",
            "1.0.231-tabbyrs.1.preview",
            "1.0.231-tabbyrs.1.nightly.20260299.1",
            "1.0.231-tabbyrs.1.nightly.20260229.1",
        ] {
            assert!(Version::parse(value).is_err(), "{value}");
        }
    }
}
