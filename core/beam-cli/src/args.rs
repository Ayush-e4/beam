//! The command line: a command, `--flag value` options and positional arguments.

use std::collections::HashMap;

/// Options that take no value.
const SWITCHES: &[&str] = &["accept-all"];

/// A parsed command line.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Args {
    /// The command, such as `pair`; empty when none was given.
    pub command: String,
    /// Arguments that aren't options, in order.
    pub positional: Vec<String>,
    options: HashMap<String, Vec<String>>,
}

impl Args {
    /// Parses `words` (without the program name).
    ///
    /// # Errors
    ///
    /// Says which option is missing its value.
    pub fn parse(words: impl IntoIterator<Item = String>) -> Result<Self, String> {
        let mut args = Self::default();
        let mut words = words.into_iter();
        while let Some(word) = words.next() {
            if let Some(name) = word.strip_prefix("--") {
                let value = if SWITCHES.contains(&name) {
                    String::new()
                } else {
                    words.next().ok_or(format!("--{name} needs a value"))?
                };
                args.options.entry(name.to_owned()).or_default().push(value);
            } else if args.command.is_empty() {
                args.command = word;
            } else {
                args.positional.push(word);
            }
        }
        Ok(args)
    }

    /// The last value given for `--name`.
    #[must_use]
    pub fn option(&self, name: &str) -> Option<&str> {
        self.options.get(name)?.last().map(String::as_str)
    }

    /// Every value given for `--name`, in order.
    #[must_use]
    pub fn all(&self, name: &str) -> &[String] {
        self.options.get(name).map_or(&[], Vec::as_slice)
    }

    /// Whether the switch `--name` was given.
    #[must_use]
    pub fn has(&self, name: &str) -> bool {
        self.options.contains_key(name)
    }

    /// `--name` as a port, or `default`.
    ///
    /// # Errors
    ///
    /// Says the value isn't a port.
    pub fn port(&self, name: &str, default: u16) -> Result<u16, String> {
        self.option(name).map_or(Ok(default), |value| {
            value
                .parse()
                .map_err(|_| format!("--{name} {value} is not a port"))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(line: &str) -> Result<Args, String> {
        Args::parse(line.split_whitespace().map(str::to_owned))
    }

    #[test]
    fn commands_options_and_positionals() -> Result<(), String> {
        let args = parse("send Pixel 192.168.1.5 --port 4000 a.jpg --address x --address y b.jpg")?;
        assert_eq!(args.command, "send");
        assert_eq!(args.positional, ["Pixel", "192.168.1.5", "a.jpg", "b.jpg"]);
        assert_eq!(args.port("port", 0)?, 4000);
        assert_eq!(args.all("address"), ["x", "y"]);
        assert!(!args.has("accept-all"));
        Ok(())
    }

    #[test]
    fn switches_take_no_value() -> Result<(), String> {
        let args = parse("receive --accept-all --into inbox")?;
        assert!(args.has("accept-all"));
        assert_eq!(args.option("into"), Some("inbox"));
        Ok(())
    }

    #[test]
    fn bad_options_are_explained() {
        assert_eq!(parse("pair --name"), Err("--name needs a value".into()));
        let not_a_port = parse("receive --port lots").and_then(|args| args.port("port", 0));
        assert_eq!(not_a_port, Err("--port lots is not a port".into()));
    }
}
