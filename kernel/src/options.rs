use alloc::collections::btree_map::BTreeMap;
use alloc::collections::btree_set::BTreeSet;

use crate::log;
use crate::sync::UninterruptibleSpinlock;
use crate::util::OneShotManualInit;

// TODO When we switch to a bootloader that allows it, we should allow options to be set from it
pub static OPTIONS_STR: &'static str = env!("HYDROXOS_OPTIONS");
static OPTIONS: OneShotManualInit<KernelOptions<'static>> = OneShotManualInit::uninit();

pub struct KernelOptions<'a> {
    options: BTreeMap<&'a str, Option<&'a str>>,
    warned_invalid: UninterruptibleSpinlock<BTreeSet<&'a str>>,
}

impl<'a> KernelOptions<'a> {
    pub fn new(mut s: &'a str) -> Self {
        let mut options = BTreeMap::new();

        s = s.trim_start();

        while !s.is_empty() {
            let key_end = s.find(|c: char| c.is_whitespace() || c == '=').unwrap_or(s.len());
            let key = &s[..key_end];

            let val = if s[key_end..].chars().next() == Some('=') {
                s = &s[key_end + 1..];

                let val = match s.chars().next() {
                    Some(quote @ ('"' | '\'')) => {
                        s = &s[1..];

                        // TODO Should we add handling for escape characters?
                        let val_end = s.find(quote).unwrap_or(s.len());

                        let val = &s[..val_end];
                        s = &s[val_end + 1..];

                        val
                    },
                    _ => {
                        let val_end = s.find(|c: char| c.is_whitespace()).unwrap_or(s.len());

                        let val = &s[..val_end];
                        s = &s[val_end..];

                        val
                    },
                };

                Some(val)
            } else {
                s = &s[key_end..];
                None
            };

            options.insert(key, val);
            s = s.trim_start();
        }

        KernelOptions {
            options,
            warned_invalid: UninterruptibleSpinlock::new(BTreeSet::new()),
        }
    }

    pub fn try_get<'b, T: KernelOptionParseable<'b>>(&'b self, key: &str) -> Option<Option<Result<T, InvalidOptionValue>>> {
        self.options.get(key).map(|val| val.map(|val| T::try_parse_kopt(val)))
    }

    pub fn warn_invalid(key: &str) {
        log!(Warning, "options", "Invalid value given for option '{}'", key);
    }

    pub fn warn_invalid_once(&self, key: &str) {
        let key = *self.options.get_key_value(key).expect("unset key to warn_invalid_once").0;

        if self.warned_invalid.lock().insert(key) {
            Self::warn_invalid(key);
        }
    }

    pub fn get<'b, T: KernelOptionParseable<'b>>(&'b self, key: &str) -> Option<T> {
        match self.try_get(key) {
            Some(Some(Ok(val))) => Some(val),
            Some(_) => {
                self.warn_invalid_once(key);
                None
            },
            None => None,
        }
    }

    pub fn get_flag<'b>(&'b self, key: &str) -> Option<bool> {
        match self.try_get(key) {
            Some(Some(Ok(val))) => Some(val),
            Some(None) => Some(true),
            Some(Some(Err(_))) => {
                self.warn_invalid_once(key);
                None
            },
            None => None,
        }
    }

    pub fn iter<'b>(&'b self) -> impl Iterator<Item = (&'b str, Option<&'b str>)> {
        self.options.iter().map(|(&k, &v)| (k, v))
    }

    pub fn iter_group<'b: 'a, T: KernelOptionParseable<'b>>(&'b self, group: &'b str) -> impl Iterator<Item = (&'b str, Option<T>)> {
        self.iter().filter_map(move |(k, v)| {
            if k.starts_with(group) {
                let sk = &k[group.len()..];

                if sk.starts_with('.') {
                    let v = match v.map(|v| T::try_parse_kopt(v)) {
                        Some(Ok(v)) => Some(v),
                        _ => {
                            self.warn_invalid_once(k);
                            None
                        },
                    };

                    Some((&sk[1..], v))
                } else {
                    None
                }
            } else {
                None
            }
        })
    }
}

pub struct InvalidOptionValue;

pub trait KernelOptionParseable<'a>
where
    Self: Sized,
{
    fn try_parse_kopt(s: &'a str) -> Result<Self, InvalidOptionValue>;
}

impl<'a> KernelOptionParseable<'a> for &'a str {
    fn try_parse_kopt(s: &'a str) -> Result<Self, InvalidOptionValue> {
        Ok(s)
    }
}

impl<'a> KernelOptionParseable<'a> for u32 {
    fn try_parse_kopt(s: &'a str) -> Result<Self, InvalidOptionValue> {
        s.parse().map_err(|_| InvalidOptionValue)
    }
}

impl<'a> KernelOptionParseable<'a> for i32 {
    fn try_parse_kopt(s: &'a str) -> Result<Self, InvalidOptionValue> {
        s.parse().map_err(|_| InvalidOptionValue)
    }
}

impl<'a> KernelOptionParseable<'a> for u64 {
    fn try_parse_kopt(s: &'a str) -> Result<Self, InvalidOptionValue> {
        s.parse().map_err(|_| InvalidOptionValue)
    }
}

impl<'a> KernelOptionParseable<'a> for i64 {
    fn try_parse_kopt(s: &'a str) -> Result<Self, InvalidOptionValue> {
        s.parse().map_err(|_| InvalidOptionValue)
    }
}

impl<'a> KernelOptionParseable<'a> for usize {
    fn try_parse_kopt(s: &'a str) -> Result<Self, InvalidOptionValue> {
        s.parse().map_err(|_| InvalidOptionValue)
    }
}

impl<'a> KernelOptionParseable<'a> for isize {
    fn try_parse_kopt(s: &'a str) -> Result<Self, InvalidOptionValue> {
        s.parse().map_err(|_| InvalidOptionValue)
    }
}

impl<'a> KernelOptionParseable<'a> for bool {
    fn try_parse_kopt(s: &'a str) -> Result<Self, InvalidOptionValue> {
        match s {
            "0" => Ok(false),
            "false" => Ok(false),
            "1" => Ok(true),
            "true" => Ok(true),
            "no" => Ok(false),
            "yes" => Ok(true),
            _ => Err(InvalidOptionValue),
        }
    }
}

pub(crate) fn init() {
    OPTIONS.set(KernelOptions::new(OPTIONS_STR));
}

pub fn get() -> &'static KernelOptions<'static> {
    OPTIONS.get()
}
