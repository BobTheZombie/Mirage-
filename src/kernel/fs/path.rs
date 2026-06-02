//! Heap-free path validation and iteration helpers.

pub const MAX_PATH_BYTES: usize = 128;
pub const MAX_COMPONENT_BYTES: usize = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PathError {
    Empty,
    TooLong,
    ComponentTooLong,
    NotAbsolute,
    InvalidByte,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Path<'a> {
    raw: &'a str,
}

impl<'a> Path<'a> {
    pub fn new(raw: &'a str) -> Result<Self, PathError> {
        Self::validate(raw, true)
    }

    pub fn new_unchecked_rooted(raw: &'a str) -> Result<Self, PathError> {
        Self::validate(raw, false)
    }

    fn validate(raw: &'a str, require_absolute: bool) -> Result<Self, PathError> {
        if raw.is_empty() {
            return Err(PathError::Empty);
        }
        if raw.len() > MAX_PATH_BYTES {
            return Err(PathError::TooLong);
        }
        if require_absolute && !raw.starts_with('/') {
            return Err(PathError::NotAbsolute);
        }

        let mut component_len = 0usize;
        for byte in raw.bytes() {
            match byte {
                0 => return Err(PathError::InvalidByte),
                b'/' => component_len = 0,
                _ => {
                    component_len += 1;
                    if component_len > MAX_COMPONENT_BYTES {
                        return Err(PathError::ComponentTooLong);
                    }
                }
            }
        }

        Ok(Self { raw })
    }

    pub const fn as_str(self) -> &'a str {
        self.raw
    }

    pub const fn is_root(self) -> bool {
        self.raw.len() == 1 && self.raw.as_bytes()[0] == b'/'
    }

    pub const fn is_absolute(self) -> bool {
        self.raw.as_bytes()[0] == b'/'
    }

    pub const fn components(self) -> Components<'a> {
        Components {
            remaining: self.raw,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Components<'a> {
    remaining: &'a str,
}

impl<'a> Components<'a> {
    pub fn next(&mut self) -> Option<&'a str> {
        while self.remaining.starts_with('/') {
            self.remaining = &self.remaining[1..];
        }
        if self.remaining.is_empty() {
            return None;
        }
        if let Some(index) = self.remaining.find('/') {
            let component = &self.remaining[..index];
            self.remaining = &self.remaining[index..];
            Some(component)
        } else {
            let component = self.remaining;
            self.remaining = "";
            Some(component)
        }
    }
}
