// Copyright 2014 The Rust Project Developers. See the COPYRIGHT
// file at the top-level directory of this distribution and at
// http://rust-lang.org/COPYRIGHT.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

// Note: adapted for SAFE Network usage from rust-glob.  See
//   https://docs.rs/glob/0.3.0/glob/
//   https://github.com/rust-lang-nursery/glob
//
// This SAFE Network adaptation utilizes a safe-api FilesMap in
// place of a regular (disk-based) FileSystem.
//
// Also, safe-api errors are used in place of std::io::Error.
//
// As a result, the API is modified from the original rust-glob
// API, but it remains similar/close.
//
// This code has been left as close to the original as possible,
// eg using #[allow] to work around clippy complaints rather
// than refactoring, keeping some dead code, etc.
//
// End Note

//! Support for matching file paths against Unix shell style patterns.
//!
//! The `glob` and `glob_with` functions allow querying a FilesMap for all
//! files that match a particular pattern (similar to the libc `glob` function).
//! The methods on the `Pattern` type provide functionality for checking if
//! individual paths match a particular pattern (similar to the libc `fnmatch`
//! function).
//!
//! For consistency across platforms, and for Windows support, this module
//! is implemented entirely in Rust rather than deferring to the libc
//! `glob`/`fnmatch` functions.
//!
//! # Examples
//!
//! To print all jpg files in `/media/` and all of its subdirectories.
//!
//! ```rust,no_run,ignore
//! use glob::glob;
//!
//! for entry in glob("/media/**/*.jpg").expect("Failed to read glob pattern") {
//!     match entry {
//!         Ok(path) => println!("{:?}", path.display()),
//!         Err(e) => println!("{:?}", e),
//!     }
//! }
//! ```
//!
//! To print all files containing the letter "a", case insensitive, in a `local`
//! directory relative to the current working directory. This ignores errors
//! instead of printing them.
//!
//! ```rust,no_run,ignore
//! use glob::glob_with;
//! use glob::MatchOptions;
//!
//! let options = MatchOptions {
//!     case_sensitive: false,
//!     require_literal_separator: false,
//!     require_literal_leading_dot: false,
//!     follow_links: false,
//! };
//! for entry in glob_with("local/*a*", &options).unwrap() {
//!     if let Ok(path) = entry {
//!         println!("{:?}", path.display())
//!     }
//! }
//! ```

#![doc(html_logo_url = "https://www.rust-lang.org/logos/rust-logo-128x128-blk-v2.png",
       html_favicon_url = "https://www.rust-lang.org/favicon.ico",
       html_root_url = "https://doc.rust-lang.org/glob/")]
#![deny(missing_docs)]
#![cfg_attr(all(test, windows), feature(std_misc))]

use super::files::FilesMap;
use super::realpath::RealPath;
use crate::Error as ApiError;

use std::cmp;
use std::error::Error;
use std::fmt;
use std::path::{self, Component, Path, PathBuf};
use std::str::FromStr;

use CharSpecifier::{CharRange, SingleChar};
use MatchResult::{EntirePatternDoesntMatch, Match, SubPatternDoesntMatch};
use PatternToken::AnyExcept;
use PatternToken::{AnyChar, AnyRecursiveSequence, AnySequence, AnyWithin, Char};

/// An iterator that yields `Path`s from the filesystem that match a particular
/// pattern.
///
/// Note that it yields `GlobResult` in order to report any `IoErrors` that may
/// arise during iteration. If a directory matches but is unreadable,
/// thereby preventing its contents from being checked for matches, a
/// `GlobError` is returned to express this.
///
/// See the `glob` function for more details.
#[derive(Debug)]
pub struct Paths<'a> {
    files_map: &'a FilesMap,
    dir_patterns: Vec<Pattern>,
    require_dir: bool,
    options: MatchOptions,
    todo: Vec<Result<(PathBuf, usize), GlobError>>,
    scope: Option<PathBuf>,
}

/// Return an iterator that produces all the `Path`s that match the given
/// pattern using default match options, which may be absolute or relative to
/// the current working directory.
///
/// This may return an error if the pattern is invalid.
///
/// This method uses the default match options and is equivalent to calling
/// `glob_with(pattern, MatchOptions::new())`. Use `glob_with` directly if you
/// want to use non-default match options.
///
/// When iterating, each result is a `GlobResult` which expresses the
/// possibility that there was an `IoError` when attempting to read the contents
/// of the matched path.  In other words, each item returned by the iterator
/// will either be an `Ok(Path)` if the path matched, or an `Err(GlobError)` if
/// the path (partially) matched _but_ its contents could not be read in order
/// to determine if its contents matched.
///
/// See the `Paths` documentation for more information.
///
/// # Examples
///
/// Consider a directory `/` containing only the files
/// `kittens.jpg`, `puppies.jpg` and `hamsters.gif`:
///
/// ```rust,no_run,ignore
/// use safe_api::glob::glob;
/// use safe_api::files::{FilesMap, FileMeta};
///
/// let meta = FileMeta::from_type_and_size("image/jpeg", "4096");
/// let fi = meta.to_file_item();
/// let mut fm = FilesMap::default();
/// fm.insert("/kittens.jpg".to_string(), fi.clone());
/// fm.insert("/dogs.jpg".to_string(), fi.clone());
/// fm.insert("/hamsters.gif".to_string(), fi.clone());
///
/// for entry in glob(&fm, "/*.jpg").unwrap() {
///     match entry {
///         Ok(path) => println!("{:?}", path.display()),
///
///         // if the path matched but was unreadable,
///         // thereby preventing its contents from matching
///         Err(e) => println!("{:?}", e),
///     }
/// }
/// ```
///
/// The above code will print:
///
/// ```ignore
/// /media/pictures/kittens.jpg
/// /media/pictures/puppies.jpg
/// ```
///
/// If you want to ignore unreadable paths, you can use something like
/// `filter_map`:
///
/// ```rust,ignore
/// use glob::glob;
/// use std::result::Result;
///
/// for path in glob("/media/pictures/*.jpg").unwrap().filter_map(Result::ok) {
///     println!("{}", path.display());
/// }
/// ```
/// Paths are yielded in alphabetical order.
pub fn glob<'a>(files_map: &'a FilesMap, pattern: &str) -> Result<Paths<'a>, PatternError> {
    glob_with(files_map, pattern, &MatchOptions::new())
}

/// Return an iterator that produces all the `Path`s that match the given
/// pattern using the specified match options, which may be absolute or relative
/// to the current working directory.
///
/// This may return an error if the pattern is invalid.
///
/// This function accepts Unix shell style patterns as described by
/// `Pattern::new(..)`.  The options given are passed through unchanged to
/// `Pattern::matches_with(..)` with the exception that
/// `require_literal_separator` is always set to `true` regardless of the value
/// passed to this function.
///
/// Paths are yielded in alphabetical order.
pub fn glob_with<'a>(
    files_map: &'a FilesMap,
    pattern: &str,
    options: &MatchOptions,
) -> Result<Paths<'a>, PatternError> {
    // make sure that the pattern is valid first, else early return with error
    let _compiled = Pattern::new(pattern)?;

    #[cfg(windows)]
    fn check_windows_verbatim(p: &Path) -> bool {
        use std::path::Prefix;
        match p.components().next() {
            Some(Component::Prefix(ref p)) => p.kind().is_verbatim(),
            _ => false,
        }
    }
    #[cfg(not(windows))]
    fn check_windows_verbatim(_: &Path) -> bool {
        false
    }

    #[cfg(windows)]
    fn to_scope(p: &Path) -> PathBuf {
        // FIXME handle volume relative paths here
        p.to_path_buf()
    }
    #[cfg(not(windows))]
    fn to_scope(p: &Path) -> PathBuf {
        p.to_path_buf()
    }

    let mut components = Path::new(pattern).components().peekable();
    #[allow(clippy::while_let_loop)]
    loop {
        match components.peek() {
            Some(&Component::Prefix(..)) | Some(&Component::RootDir) => {
                components.next();
            }
            _ => break,
        }
    }
    let rest = components.map(|s| s.as_os_str()).collect::<PathBuf>();
    let normalized_pattern = Path::new(pattern).iter().collect::<PathBuf>();
    let root_len = normalized_pattern.to_str().unwrap().len() - rest.to_str().unwrap().len();
    let root = if root_len > 0 {
        Some(Path::new(&pattern[..root_len]))
    } else {
        None
    };

    if root_len > 0 && check_windows_verbatim(root.unwrap()) {
        // FIXME: How do we want to handle verbatim paths? I'm inclined to
        // return nothing, since we can't very well find all UNC shares with a
        // 1-letter server name.
        return Ok(Paths {
            files_map,
            dir_patterns: Vec::new(),
            require_dir: false,
            options: options.clone(),
            todo: Vec::new(),
            scope: None,
        });
    }

    let scope = root.map(to_scope).unwrap_or_else(|| PathBuf::from("."));

    let mut dir_patterns = Vec::new();
    let components =
        pattern[cmp::min(root_len, pattern.len())..].split_terminator(path::is_separator);

    for component in components {
        let compiled = Pattern::new(component)?;
        dir_patterns.push(compiled);
    }

    if root_len == pattern.len() {
        dir_patterns.push(Pattern {
            original: "".to_string(),
            tokens: Vec::new(),
            is_recursive: false,
        });
    }

    let last_is_separator = pattern.chars().next_back().map(path::is_separator);
    let require_dir = last_is_separator == Some(true);
    let todo = Vec::new();

    Ok(Paths {
        files_map,
        dir_patterns,
        require_dir,
        options: options.clone(),
        todo,
        scope: Some(scope),
    })
}

/// A glob iteration error.
///
/// This is typically returned when a particular path cannot be read
/// to determine if its contents match the glob pattern. This is possible
/// if the program lacks the appropriate permissions, for example.
#[derive(Debug)]
pub struct GlobError {
    path: PathBuf,
    error: ApiError,
}

impl GlobError {
    /// The error in question.
    pub fn error(&self) -> &ApiError {
        &self.error
    }
}

impl Error for GlobError {}

impl fmt::Display for GlobError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "attempting to read `{}` resulted in an error: {}",
            self.path.display(),
            self.error
        )
    }
}

fn is_dir(files_map: &FilesMap, p: &Path, follow_links: bool) -> bool {
    let path_str = p.display().to_string();
    if path_str == "/" {
        return true;
    }

    let path = if follow_links {
        match files_map.realpath(&path_str) {
            Ok(p) => p,
            Err(_) => return false,
        }
    } else {
        path_str
    };
    let fi = match files_map.get(&path) {
        Some(f) => f,
        None => return false,
    };
    match fi.get("type") {
        Some(s) => s == "inode/directory",
        _ => false,
    }
}

/// An alias for a glob iteration result.
///
/// This represents either a matched path or a glob iteration error,
/// such as failing to read a particular directory's contents.
pub type GlobResult = Result<PathBuf, GlobError>;

impl<'a> Iterator for Paths<'a> {
    type Item = GlobResult;

    fn next(&mut self) -> Option<GlobResult> {
        // the todo buffer hasn't been initialized yet, so it's done at this
        // point rather than in glob() so that the errors are unified that is,
        // failing to fill the buffer is an iteration error construction of the
        // iterator (i.e. glob()) only fails if it fails to compile the Pattern
        if let Some(scope) = self.scope.take() {
            if !self.dir_patterns.is_empty() {
                // Shouldn't happen, but we're using -1 as a special index.
                assert!(self.dir_patterns.len() < !0 as usize);

                fill_todo(
                    self.files_map,
                    &mut self.todo,
                    &self.dir_patterns,
                    0,
                    &scope,
                    &self.options,
                );
            }
        }

        loop {
            if self.dir_patterns.is_empty() || self.todo.is_empty() {
                return None;
            }

            let (path, mut idx) = match self.todo.pop().unwrap() {
                Ok(pair) => pair,
                Err(e) => return Some(Err(e)),
            };

            // idx -1: was already checked by fill_todo, maybe path was '.' or
            // '..' that we can't match here because of normalization.
            if idx == !0 as usize {
                if self.require_dir && !is_dir(self.files_map, &path, self.options.follow_links) {
                    continue;
                }
                return Some(Ok(path));
            }

            if self.dir_patterns[idx].is_recursive {
                let mut next = idx;

                // collapse consecutive recursive patterns
                while (next + 1) < self.dir_patterns.len()
                    && self.dir_patterns[next + 1].is_recursive
                {
                    next += 1;
                }

                if is_dir(self.files_map, &path, self.options.follow_links) {
                    // the path is a directory, so it's a match

                    // push this directory's contents
                    fill_todo(
                        self.files_map,
                        &mut self.todo,
                        &self.dir_patterns,
                        next,
                        &path,
                        &self.options,
                    );

                    if next == self.dir_patterns.len() - 1 {
                        // pattern ends in recursive pattern, so return this
                        // directory as a result
                        return Some(Ok(path));
                    } else {
                        // advanced to the next pattern for this path
                        idx = next + 1;
                    }
                } else if next != self.dir_patterns.len() - 1 {
                    // advanced to the next pattern for this path
                    idx = next + 1;
                } else {
                    // not a directory and it's the last pattern, meaning no
                    // match
                    continue;
                }
            }

            // not recursive, so match normally
            if self.dir_patterns[idx].matches_with(
                {
                    match path.file_name().and_then(|s| s.to_str()) {
                        // FIXME (#9639): How do we handle non-utf8 filenames?
                        // Ignore them for now; ideally we'd still match them
                        // against a *
                        None => continue,
                        Some(x) => x,
                    }
                },
                &self.options,
            ) {
                if idx == self.dir_patterns.len() - 1 {
                    // it is not possible for a pattern to match a directory
                    // *AND* its children so we don't need to check the
                    // children

                    if !self.require_dir || is_dir(self.files_map, &path, self.options.follow_links)
                    {
                        return Some(Ok(path));
                    }
                } else {
                    fill_todo(
                        self.files_map,
                        &mut self.todo,
                        &self.dir_patterns,
                        idx + 1,
                        &path,
                        &self.options,
                    );
                }
            }
        }
    }
}

/// A pattern parsing error.
#[derive(Debug)]
#[allow(missing_copy_implementations)]
pub struct PatternError {
    /// The approximate character index of where the error occurred.
    pub pos: usize,

    /// A message describing the error.
    pub msg: &'static str,
}

impl Error for PatternError {
    fn description(&self) -> &str {
        self.msg
    }
}

impl fmt::Display for PatternError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "Pattern syntax error near position {}: {}",
            self.pos, self.msg
        )
    }
}

/// A compiled Unix shell style pattern.
///
/// - `?` matches any single character.
///
/// - `*` matches any (possibly empty) sequence of characters.
///
/// - `**` matches the current directory and arbitrary subdirectories. This
///   sequence **must** form a single path component, so both `**a` and `b**`
///   are invalid and will result in an error.  A sequence of more than two
///   consecutive `*` characters is also invalid.
///
/// - `[...]` matches any character inside the brackets.  Character sequences
///   can also specify ranges of characters, as ordered by Unicode, so e.g.
///   `[0-9]` specifies any character between 0 and 9 inclusive. An unclosed
///   bracket is invalid.
///
/// - `[!...]` is the negation of `[...]`, i.e. it matches any characters
///   **not** in the brackets.
///
/// - The metacharacters `?`, `*`, `[`, `]` can be matched by using brackets
///   (e.g. `[?]`).  When a `]` occurs immediately following `[` or `[!` then it
///   is interpreted as being part of, rather then ending, the character set, so
///   `]` and NOT `]` can be matched by `[]]` and `[!]]` respectively.  The `-`
///   character can be specified inside a character sequence pattern by placing
///   it at the start or the end, e.g. `[abc-]`.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Debug)]
pub struct Pattern {
    original: String,
    tokens: Vec<PatternToken>,
    is_recursive: bool,
}

/// Show the original glob pattern.
impl fmt::Display for Pattern {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        self.original.fmt(f)
    }
}

impl FromStr for Pattern {
    type Err = PatternError;

    fn from_str(s: &str) -> Result<Pattern, PatternError> {
        Pattern::new(s)
    }
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
enum PatternToken {
    Char(char),
    AnyChar,
    AnySequence,
    AnyRecursiveSequence,
    AnyWithin(Vec<CharSpecifier>),
    AnyExcept(Vec<CharSpecifier>),
}

#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
enum CharSpecifier {
    SingleChar(char),
    CharRange(char, char),
}

#[derive(Copy, Clone, PartialEq)]
enum MatchResult {
    Match,
    SubPatternDoesntMatch,
    EntirePatternDoesntMatch,
}

const ERROR_WILDCARDS: &str = "wildcards are either regular `*` or recursive `**`";
const ERROR_RECURSIVE_WILDCARDS: &str = "recursive wildcards must form a single path \
                                                 component";
const ERROR_INVALID_RANGE: &str = "invalid range pattern";

impl Pattern {
    /// This function compiles Unix shell style patterns.
    ///
    /// An invalid glob pattern will yield a `PatternError`.
    #[allow(clippy::comparison_chain)]
    pub fn new(pattern: &str) -> Result<Pattern, PatternError> {
        let chars = pattern.chars().collect::<Vec<_>>();
        let mut tokens = Vec::new();
        let mut is_recursive = false;
        let mut i = 0;

        while i < chars.len() {
            match chars[i] {
                '?' => {
                    tokens.push(AnyChar);
                    i += 1;
                }
                '*' => {
                    let old = i;

                    while i < chars.len() && chars[i] == '*' {
                        i += 1;
                    }

                    let count = i - old;

                    if count > 2 {
                        return Err(PatternError {
                            pos: old + 2,
                            msg: ERROR_WILDCARDS,
                        });
                    } else if count == 2 {
                        // ** can only be an entire path component
                        // i.e. a/**/b is valid, but a**/b or a/**b is not
                        // invalid matches are treated literally
                        let is_valid = if i == 2 || path::is_separator(chars[i - count - 1]) {
                            // it ends in a '/'
                            if i < chars.len() && path::is_separator(chars[i]) {
                                i += 1;
                                true
                            // or the pattern ends here
                            // this enables the existing globbing mechanism
                            } else if i == chars.len() {
                                true
                            // `**` ends in non-separator
                            } else {
                                return Err(PatternError {
                                    pos: i,
                                    msg: ERROR_RECURSIVE_WILDCARDS,
                                });
                            }
                        // `**` begins with non-separator
                        } else {
                            return Err(PatternError {
                                pos: old - 1,
                                msg: ERROR_RECURSIVE_WILDCARDS,
                            });
                        };

                        let tokens_len = tokens.len();

                        if is_valid {
                            // collapse consecutive AnyRecursiveSequence to a
                            // single one
                            if !(tokens_len > 1 && tokens[tokens_len - 1] == AnyRecursiveSequence) {
                                is_recursive = true;
                                tokens.push(AnyRecursiveSequence);
                            }
                        }
                    } else {
                        tokens.push(AnySequence);
                    }
                }
                '[' => {
                    if i + 4 <= chars.len() && chars[i + 1] == '!' {
                        match chars[i + 3..].iter().position(|x| *x == ']') {
                            None => (),
                            Some(j) => {
                                let chars = &chars[i + 2..i + 3 + j];
                                let cs = parse_char_specifiers(chars);
                                tokens.push(AnyExcept(cs));
                                i += j + 4;
                                continue;
                            }
                        }
                    } else if i + 3 <= chars.len() && chars[i + 1] != '!' {
                        match chars[i + 2..].iter().position(|x| *x == ']') {
                            None => (),
                            Some(j) => {
                                let cs = parse_char_specifiers(&chars[i + 1..i + 2 + j]);
                                tokens.push(AnyWithin(cs));
                                i += j + 3;
                                continue;
                            }
                        }
                    }

                    // if we get here then this is not a valid range pattern
                    return Err(PatternError {
                        pos: i,
                        msg: ERROR_INVALID_RANGE,
                    });
                }
                c => {
                    tokens.push(Char(c));
                    i += 1;
                }
            }
        }

        Ok(Pattern {
            tokens,
            original: pattern.to_string(),
            is_recursive,
        })
    }

    pub fn is_plain_path(path: &str) -> bool {
        fn is_pattern_char(c: char) -> bool {
            match c {
                '*' => true,
                '?' => true,
                '[' => true,
                ']' => true,
                _ => false,
            }
        }
        !path.contains(is_pattern_char)
    }

    /// Escape metacharacters within the given string by surrounding them in
    /// brackets. The resulting string will, when compiled into a `Pattern`,
    /// match the input string and nothing else.
    #[allow(dead_code)]
    pub fn escape(s: &str) -> String {
        let mut escaped = String::new();
        for c in s.chars() {
            match c {
                // note that ! does not need escaping because it is only special
                // inside brackets
                '?' | '*' | '[' | ']' => {
                    escaped.push('[');
                    escaped.push(c);
                    escaped.push(']');
                }
                c => {
                    escaped.push(c);
                }
            }
        }
        escaped
    }

    /// Return if the given `str` matches this `Pattern` using the default
    /// match options (i.e. `MatchOptions::new()`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// use glob::Pattern;
    ///
    /// assert!(Pattern::new("c?t").unwrap().matches("cat"));
    /// assert!(Pattern::new("k[!e]tteh").unwrap().matches("kitteh"));
    /// assert!(Pattern::new("d*g").unwrap().matches("doog"));
    /// ```
    #[allow(dead_code)]
    pub fn matches(&self, str: &str) -> bool {
        self.matches_with(str, &MatchOptions::new())
    }

    /// Return if the given `Path`, when converted to a `str`, matches this
    /// `Pattern` using the default match options (i.e. `MatchOptions::new()`).
    #[allow(dead_code)]
    pub fn matches_path(&self, path: &Path) -> bool {
        // FIXME (#9639): This needs to handle non-utf8 paths
        path.to_str().map_or(false, |s| self.matches(s))
    }

    /// Return if the given `str` matches this `Pattern` using the specified
    /// match options.
    pub fn matches_with(&self, str: &str, options: &MatchOptions) -> bool {
        self.matches_from(true, str.chars(), 0, options) == Match
    }

    fn matches_from(
        &self,
        mut follows_separator: bool,
        mut file: std::str::Chars,
        i: usize,
        options: &MatchOptions,
    ) -> MatchResult {
        for (ti, token) in self.tokens[i..].iter().enumerate() {
            match *token {
                AnySequence | AnyRecursiveSequence => {
                    // ** must be at the start.
                    debug_assert!(match *token {
                        AnyRecursiveSequence => follows_separator,
                        _ => true,
                    });

                    // Empty match
                    match self.matches_from(follows_separator, file.clone(), i + ti + 1, options) {
                        SubPatternDoesntMatch => (), // keep trying
                        m => return m,
                    };

                    while let Some(c) = file.next() {
                        if follows_separator && options.require_literal_leading_dot && c == '.' {
                            return SubPatternDoesntMatch;
                        }
                        follows_separator = path::is_separator(c);
                        match *token {
                            AnyRecursiveSequence if !follows_separator => continue,
                            AnySequence
                                if options.require_literal_separator && follows_separator =>
                            {
                                return SubPatternDoesntMatch
                            }
                            _ => (),
                        }
                        match self.matches_from(
                            follows_separator,
                            file.clone(),
                            i + ti + 1,
                            options,
                        ) {
                            SubPatternDoesntMatch => (), // keep trying
                            m => return m,
                        }
                    }
                }
                _ => {
                    let c = match file.next() {
                        Some(c) => c,
                        None => return EntirePatternDoesntMatch,
                    };

                    let is_sep = path::is_separator(c);

                    if !match *token {
                        AnyChar | AnyWithin(..) | AnyExcept(..)
                            if (options.require_literal_separator && is_sep)
                                || (follows_separator
                                    && options.require_literal_leading_dot
                                    && c == '.') =>
                        {
                            false
                        }
                        AnyChar => true,
                        AnyWithin(ref specifiers) => in_char_specifiers(&specifiers, c, options),
                        AnyExcept(ref specifiers) => !in_char_specifiers(&specifiers, c, options),
                        Char(c2) => chars_eq(c, c2, options.case_sensitive),
                        AnySequence | AnyRecursiveSequence => unreachable!(),
                    } {
                        return SubPatternDoesntMatch;
                    }
                    follows_separator = is_sep;
                }
            }
        }

        // Iter is fused.
        if file.next().is_none() {
            Match
        } else {
            SubPatternDoesntMatch
        }
    }
}

// Fills `todo` with paths under `path` to be matched by `patterns[idx]`,
// special-casing patterns to match `.` and `..`, and avoiding `readdir()`
// calls when there are no metacharacters in the pattern.
fn fill_todo(
    files_map: &FilesMap,
    todo: &mut Vec<Result<(PathBuf, usize), GlobError>>,
    patterns: &[Pattern],
    idx: usize,
    path: &Path,
    options: &MatchOptions,
) {
    // convert a pattern that's just many Char(_) to a string
    fn pattern_as_str(pattern: &Pattern) -> Option<String> {
        let mut s = String::new();
        for token in pattern.tokens.iter() {
            match *token {
                Char(c) => s.push(c),
                _ => return None,
            }
        }
        Some(s)
    }

    let add = |todo: &mut Vec<_>, next_path: PathBuf| {
        if idx + 1 == patterns.len() {
            // We know it's good, so don't make the iterator match this path
            // against the pattern again. In particular, it can't match
            // . or .. globs since these never show up as path components.
            todo.push(Ok((next_path, !0 as usize)));
        } else {
            fill_todo(files_map, todo, patterns, idx + 1, &next_path, options);
        }
    };

    let pattern = &patterns[idx];
    let is_dir = is_dir(files_map, path, options.follow_links);
    let curdir = path == Path::new(".");
    match pattern_as_str(pattern) {
        Some(s) => {
            // This pattern component doesn't have any metacharacters, so we
            // don't need to read the current directory to know where to
            // continue. So instead of passing control back to the iterator,
            // we can just check for that one entry and potentially recurse
            // right away.
            let special = "." == s || ".." == s;
            let next_path = if curdir {
                s
            } else {
                let j = path.join(&s).display().to_string();
                match files_map.realpath(&j) {
                    Ok(p) => p,
                    Err(e) => {
                        todo.push(Err(GlobError {
                            path: Path::new(&j).to_path_buf(),
                            error: e,
                        }));
                        return;
                    }
                }
            };
            if (special && is_dir)
                || (!special && files_map.get(&next_path).is_some())
                || next_path == "/"
            {
                add(todo, Path::new(&next_path).to_path_buf());
            }
        }
        None if is_dir => {
            // note:  files_map_read_dir() is SAFE replacement for
            // read_dir() in original rust-glob code.
            let mut children = files_map_read_dir(files_map, &path.display().to_string());
            children.sort_by(|p1, p2| p2.file_name().cmp(&p1.file_name()));
            todo.extend(children.into_iter().map(|x| Ok((x, idx))));

            // Matching the special directory entries . and .. that
            // refer to the current and parent directory respectively
            // requires that the pattern has a leading dot, even if the
            // `MatchOptions` field `require_literal_leading_dot` is not
            // set.
            if !pattern.tokens.is_empty() && pattern.tokens[0] == Char('.') {
                for &special in [".", ".."].iter() {
                    if pattern.matches_with(special, options) {
                        add(todo, path.join(special));
                    }
                }
            }
        }
        None => {
            // not a directory, nothing more to find
        }
    }
}

// Finds direct children of a directory path in a FilesMap.
// similar to std::fs::read_dir()
//
// This is expensive/inefficient for large FilesMap because it reads all
// keys (paths) in the filesmap in order to find only those that are direct
// children of a particular directory.
//
// Further, calling code calls it for each directory component of a path.
//
// It would be much more efficient to use a tree structure rather than
// HashMap to hold the filesystem data.
fn files_map_read_dir(files_map: &FilesMap, path: &str) -> Vec<PathBuf> {
    let mut matches = Vec::<PathBuf>::new();
    for pathkey in files_map.keys() {
        let remainder = strip_prefix_or_empty(pathkey, path.trim_end_matches('/'));
        if !remainder.is_empty() && remainder.starts_with('/') {
            let target_cnt = if remainder.starts_with('/') { 2 } else { 1 };
            if remainder.split('/').count() == target_cnt {
                matches.push(Path::new(pathkey).to_path_buf());
            }
        }
    }
    matches
}

// removes prefix from s, or empty string if not found.
fn strip_prefix_or_empty(s: &str, prefix: &str) -> String {
    if s.starts_with(prefix) {
        s[prefix.len()..].to_string()
    } else {
        "".to_string()
    }
}

fn parse_char_specifiers(s: &[char]) -> Vec<CharSpecifier> {
    let mut cs = Vec::new();
    let mut i = 0;
    while i < s.len() {
        if i + 3 <= s.len() && s[i + 1] == '-' {
            cs.push(CharRange(s[i], s[i + 2]));
            i += 3;
        } else {
            cs.push(SingleChar(s[i]));
            i += 1;
        }
    }
    cs
}

fn in_char_specifiers(specifiers: &[CharSpecifier], c: char, options: &MatchOptions) -> bool {
    for &specifier in specifiers.iter() {
        match specifier {
            SingleChar(sc) => {
                if chars_eq(c, sc, options.case_sensitive) {
                    return true;
                }
            }
            CharRange(start, end) => {
                // FIXME: work with non-ascii chars properly (issue #1347)
                if !options.case_sensitive && c.is_ascii() && start.is_ascii() && end.is_ascii() {
                    let start = start.to_ascii_lowercase();
                    let end = end.to_ascii_lowercase();

                    let start_up = start.to_uppercase().next().unwrap();
                    let end_up = end.to_uppercase().next().unwrap();

                    // only allow case insensitive matching when
                    // both start and end are within a-z or A-Z
                    if start != start_up && end != end_up {
                        let c = c.to_ascii_lowercase();
                        if c >= start && c <= end {
                            return true;
                        }
                    }
                }

                if c >= start && c <= end {
                    return true;
                }
            }
        }
    }

    false
}

/// A helper function to determine if two chars are (possibly case-insensitively) equal.
fn chars_eq(a: char, b: char, case_sensitive: bool) -> bool {
    if cfg!(windows) && path::is_separator(a) && path::is_separator(b) {
        true
    } else if !case_sensitive && a.is_ascii() && b.is_ascii() {
        // FIXME: work with non-ascii chars properly (issue #9084)
        a.to_ascii_lowercase() == b.to_ascii_lowercase()
    } else {
        a == b
    }
}

/// Configuration options to modify the behaviour of `Pattern::matches_with(..)`.
#[allow(missing_copy_implementations)]
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Debug)]
pub struct MatchOptions {
    /// Whether or not patterns should be matched in a case-sensitive manner.
    /// This currently only considers upper/lower case relationships between
    /// ASCII characters, but in future this might be extended to work with
    /// Unicode.
    pub case_sensitive: bool,

    /// Whether or not path-component separator characters (e.g. `/` on
    /// Posix) must be matched by a literal `/`, rather than by `*` or `?` or
    /// `[...]`.
    pub require_literal_separator: bool,

    /// Whether or not paths that contain components that start with a `.`
    /// will require that `.` appears literally in the pattern; `*`, `?`, `**`,
    /// or `[...]` will not match. This is useful because such files are
    /// conventionally considered hidden on Unix systems and it might be
    /// desirable to skip them when listing files.
    pub require_literal_leading_dot: bool,

    /// Whether ot not paths that are symbolic links are followed
    /// during the walk.
    pub follow_links: bool,
}

impl MatchOptions {
    /// Constructs a new `MatchOptions` with default field values. This is used
    /// when calling functions that do not take an explicit `MatchOptions`
    /// parameter.
    ///
    /// This function always returns this value:
    ///
    /// ```rust,ignore
    /// MatchOptions {
    ///     case_sensitive: true,
    ///     require_literal_separator: false,
    ///     require_literal_leading_dot: false,
    ///     follow_links: false,
    /// }
    /// ```
    pub fn new() -> MatchOptions {
        MatchOptions {
            case_sensitive: true,
            require_literal_separator: false,
            require_literal_leading_dot: false,
            follow_links: false,
        }
    }
}

#[cfg(test)]
mod test {
    use super::{glob, MatchOptions, Pattern};
    use crate::files::{FileMeta, FilesMap};
    use std::path::Path;

    fn mk_filesmap() -> FilesMap {
        let meta_dir = FileMeta::from_type_and_size("inode/directory", "4096");
        let fi_dir = meta_dir.to_file_item();

        let meta = FileMeta::from_type_and_size("image/jpeg", "4096");
        let fi = meta.to_file_item();
        let mut fm = FilesMap::default();
        fm.insert("/".to_string(), fi_dir.clone());
        fm.insert("/kittens.jpg".to_string(), fi.clone());
        fm.insert("/dogs.jpg".to_string(), fi.clone());
        fm.insert("/hamsters.gif".to_string(), fi.clone());
        fm.insert("/sub".to_string(), fi_dir);
        fm.insert("/sub/a1.jpg".to_string(), fi.clone());
        fm.insert("/sub/b2.jpg".to_string(), fi.clone());
        fm.insert("/sub/c3.txt".to_string(), fi);

        fm
    }

    #[test]
    fn test_glob_pattern_from_str() {
        assert!("a*b".parse::<Pattern>().unwrap().matches("a_b"));
        assert!("a/**b".parse::<Pattern>().unwrap_err().pos == 4);
    }

    #[test]
    fn test_glob_wildcard_errors() {
        assert!(Pattern::new("a/**b").unwrap_err().pos == 4);
        assert!(Pattern::new("a/bc**").unwrap_err().pos == 3);
        assert!(Pattern::new("a/*****").unwrap_err().pos == 4);
        assert!(Pattern::new("a/b**c**d").unwrap_err().pos == 2);
        assert!(Pattern::new("a**b").unwrap_err().pos == 0);
    }

    #[test]
    fn test_glob_unclosed_bracket_errors() {
        assert!(Pattern::new("abc[def").unwrap_err().pos == 3);
        assert!(Pattern::new("abc[!def").unwrap_err().pos == 3);
        assert!(Pattern::new("abc[").unwrap_err().pos == 3);
        assert!(Pattern::new("abc[!").unwrap_err().pos == 3);
        assert!(Pattern::new("abc[d").unwrap_err().pos == 3);
        assert!(Pattern::new("abc[!d").unwrap_err().pos == 3);
        assert!(Pattern::new("abc[]").unwrap_err().pos == 3);
        assert!(Pattern::new("abc[!]").unwrap_err().pos == 3);
    }

    #[test]
    fn test_glob_errors() {
        assert!(glob(&FilesMap::default(), "a/**b").err().unwrap().pos == 4);
        assert!(glob(&FilesMap::default(), "abc[def").err().unwrap().pos == 3);
    }

    // this test assumes that there is a /root directory and that
    // the user running this test is not root or otherwise doesn't
    // have permission to read its contents
    #[cfg(unix)]
    #[test]
    #[ignore]
    fn test_glob_iteration_errors() {
        let fm = mk_filesmap();
        let mut iter = glob(&fm, "/root/*").unwrap();

        // GlobErrors shouldn't halt iteration
        let next = iter.next();
        assert!(next.is_some());

        let err = next.unwrap();
        assert!(err.is_err());

        //        let err = err.err().unwrap();
        //        assert!(err.path() == Path::new("/root"));
        //        assert!(err.error().kind() == io::ErrorKind::PermissionDenied);
    }

    #[test]
    fn test_glob_absolute_pattern() {
        let fm = mk_filesmap();
        assert!(glob(&fm, "/").unwrap().next().is_some());
        assert!(glob(&fm, "//").unwrap().next().is_some());

        // assume that the filesystem is not empty!
        let _r = glob(&fm, "/*.jpg").unwrap();
        assert!(glob(&fm, "/*.jpg").unwrap().next().is_some());

        #[cfg(not(windows))]
        fn win() {}

        #[cfg(windows)]
        fn win() {
            use std::env::current_dir;
            use std::ffi::AsOsStr;

            // check windows absolute paths with host/device components
            let root_with_device = current_dir()
                .ok()
                .and_then(|p| p.prefix().map(|p| p.join("*")))
                .unwrap();
            // FIXME (#9639): This needs to handle non-utf8 paths
            assert!(glob(&fm, root_with_device.as_os_str().to_str().unwrap())
                .unwrap()
                .next()
                .is_some());
        }
        win()
    }

    #[test]
    fn test_glob_wildcards() {
        assert!(Pattern::new("a*b").unwrap().matches("a_b"));
        assert!(Pattern::new("a*b*c").unwrap().matches("abc"));
        assert!(!Pattern::new("a*b*c").unwrap().matches("abcd"));
        assert!(Pattern::new("a*b*c").unwrap().matches("a_b_c"));
        assert!(Pattern::new("a*b*c").unwrap().matches("a___b___c"));
        assert!(Pattern::new("abc*abc*abc")
            .unwrap()
            .matches("abcabcabcabcabcabcabc"));
        assert!(!Pattern::new("abc*abc*abc")
            .unwrap()
            .matches("abcabcabcabcabcabcabca"));
        assert!(Pattern::new("a*a*a*a*a*a*a*a*a")
            .unwrap()
            .matches("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"));
        assert!(Pattern::new("a*b[xyz]c*d").unwrap().matches("abxcdbxcddd"));
    }

    #[test]
    #[allow(clippy::cognitive_complexity)]
    fn test_glob_recursive_wildcards() {
        let pat = Pattern::new("some/**/needle.txt").unwrap();
        assert!(pat.matches("some/needle.txt"));
        assert!(pat.matches("some/one/needle.txt"));
        assert!(pat.matches("some/one/two/needle.txt"));
        assert!(pat.matches("some/other/needle.txt"));
        assert!(!pat.matches("some/other/notthis.txt"));

        // a single ** should be valid, for globs
        // Should accept anything
        let pat = Pattern::new("**").unwrap();
        assert!(pat.is_recursive);
        assert!(pat.matches("abcde"));
        assert!(pat.matches(""));
        assert!(pat.matches(".asdf"));
        assert!(pat.matches("/x/.asdf"));

        // collapse consecutive wildcards
        let pat = Pattern::new("some/**/**/needle.txt").unwrap();
        assert!(pat.matches("some/needle.txt"));
        assert!(pat.matches("some/one/needle.txt"));
        assert!(pat.matches("some/one/two/needle.txt"));
        assert!(pat.matches("some/other/needle.txt"));
        assert!(!pat.matches("some/other/notthis.txt"));

        // ** can begin the pattern
        let pat = Pattern::new("**/test").unwrap();
        assert!(pat.matches("one/two/test"));
        assert!(pat.matches("one/test"));
        assert!(pat.matches("test"));

        // /** can begin the pattern
        let pat = Pattern::new("/**/test").unwrap();
        assert!(pat.matches("/one/two/test"));
        assert!(pat.matches("/one/test"));
        assert!(pat.matches("/test"));
        assert!(!pat.matches("/one/notthis"));
        assert!(!pat.matches("/notthis"));

        // Only start sub-patterns on start of path segment.
        let pat = Pattern::new("**/.*").unwrap();
        assert!(pat.matches(".abc"));
        assert!(pat.matches("abc/.abc"));
        assert!(!pat.matches("ab.c"));
        assert!(!pat.matches("abc/ab.c"));
    }

    #[test]
    #[ignore]
    fn test_glob_lots_of_files() {
        // this is a good test because it touches lots of differently named files
        #[allow(clippy::iter_skip_next)]
        glob(&mk_filesmap(), "/*/*/*/*").unwrap().skip(10000).next();
    }

    #[test]
    fn test_glob_range_pattern() {
        let pat = Pattern::new("a[0-9]b").unwrap();
        for i in 0..10 {
            assert!(pat.matches(&format!("a{}b", i)));
        }
        assert!(!pat.matches("a_b"));

        let pat = Pattern::new("a[!0-9]b").unwrap();
        for i in 0..10 {
            assert!(!pat.matches(&format!("a{}b", i)));
        }
        assert!(pat.matches("a_b"));

        let pats = ["[a-z123]", "[1a-z23]", "[123a-z]"];
        for &p in pats.iter() {
            let pat = Pattern::new(p).unwrap();
            for c in "abcdefghijklmnopqrstuvwxyz".chars() {
                assert!(pat.matches(&c.to_string()));
            }
            for c in "ABCDEFGHIJKLMNOPQRSTUVWXYZ".chars() {
                let options = MatchOptions {
                    case_sensitive: false,
                    ..MatchOptions::new()
                };
                assert!(pat.matches_with(&c.to_string(), &options));
            }
            assert!(pat.matches("1"));
            assert!(pat.matches("2"));
            assert!(pat.matches("3"));
        }

        let pats = ["[abc-]", "[-abc]", "[a-c-]"];
        for &p in pats.iter() {
            let pat = Pattern::new(p).unwrap();
            assert!(pat.matches("a"));
            assert!(pat.matches("b"));
            assert!(pat.matches("c"));
            assert!(pat.matches("-"));
            assert!(!pat.matches("d"));
        }

        let pat = Pattern::new("[2-1]").unwrap();
        assert!(!pat.matches("1"));
        assert!(!pat.matches("2"));

        assert!(Pattern::new("[-]").unwrap().matches("-"));
        assert!(!Pattern::new("[!-]").unwrap().matches("-"));
    }

    #[test]
    fn test_glob_pattern_matches() {
        let txt_pat = Pattern::new("*hello.txt").unwrap();
        assert!(txt_pat.matches("hello.txt"));
        assert!(txt_pat.matches("gareth_says_hello.txt"));
        assert!(txt_pat.matches("some/path/to/hello.txt"));
        assert!(txt_pat.matches("some\\path\\to\\hello.txt"));
        assert!(txt_pat.matches("/an/absolute/path/to/hello.txt"));
        assert!(!txt_pat.matches("hello.txt-and-then-some"));
        assert!(!txt_pat.matches("goodbye.txt"));

        let dir_pat = Pattern::new("*some/path/to/hello.txt").unwrap();
        assert!(dir_pat.matches("some/path/to/hello.txt"));
        assert!(dir_pat.matches("a/bigger/some/path/to/hello.txt"));
        assert!(!dir_pat.matches("some/path/to/hello.txt-and-then-some"));
        assert!(!dir_pat.matches("some/other/path/to/hello.txt"));
    }

    #[test]
    fn test_glob_pattern_escape() {
        let s = "_[_]_?_*_!_";
        assert_eq!(Pattern::escape(s), "_[[]_[]]_[?]_[*]_!_".to_string());
        assert!(Pattern::new(&Pattern::escape(s)).unwrap().matches(s));
    }

    #[test]
    fn test_glob_pattern_matches_case_insensitive() {
        let pat = Pattern::new("aBcDeFg").unwrap();
        let options = MatchOptions {
            case_sensitive: false,
            require_literal_separator: false,
            require_literal_leading_dot: false,
            follow_links: false,
        };

        assert!(pat.matches_with("aBcDeFg", &options));
        assert!(pat.matches_with("abcdefg", &options));
        assert!(pat.matches_with("ABCDEFG", &options));
        assert!(pat.matches_with("AbCdEfG", &options));
    }

    #[test]
    fn test_glob_pattern_matches_case_insensitive_range() {
        let pat_within = Pattern::new("[a]").unwrap();
        let pat_except = Pattern::new("[!a]").unwrap();

        let options_case_insensitive = MatchOptions {
            case_sensitive: false,
            require_literal_separator: false,
            require_literal_leading_dot: false,
            follow_links: false,
        };
        let options_case_sensitive = MatchOptions {
            case_sensitive: true,
            require_literal_separator: false,
            require_literal_leading_dot: false,
            follow_links: false,
        };

        assert!(pat_within.matches_with("a", &options_case_insensitive));
        assert!(pat_within.matches_with("A", &options_case_insensitive));
        assert!(!pat_within.matches_with("A", &options_case_sensitive));

        assert!(!pat_except.matches_with("a", &options_case_insensitive));
        assert!(!pat_except.matches_with("A", &options_case_insensitive));
        assert!(pat_except.matches_with("A", &options_case_sensitive));
    }

    #[test]
    fn test_glob_pattern_matches_require_literal_separator() {
        let options_require_literal = MatchOptions {
            case_sensitive: true,
            require_literal_separator: true,
            require_literal_leading_dot: false,
            follow_links: false,
        };
        let options_not_require_literal = MatchOptions {
            case_sensitive: true,
            require_literal_separator: false,
            require_literal_leading_dot: false,
            follow_links: false,
        };

        assert!(Pattern::new("abc/def")
            .unwrap()
            .matches_with("abc/def", &options_require_literal));
        assert!(!Pattern::new("abc?def")
            .unwrap()
            .matches_with("abc/def", &options_require_literal));
        assert!(!Pattern::new("abc*def")
            .unwrap()
            .matches_with("abc/def", &options_require_literal));
        assert!(!Pattern::new("abc[/]def")
            .unwrap()
            .matches_with("abc/def", &options_require_literal));

        assert!(Pattern::new("abc/def")
            .unwrap()
            .matches_with("abc/def", &options_not_require_literal));
        assert!(Pattern::new("abc?def")
            .unwrap()
            .matches_with("abc/def", &options_not_require_literal));
        assert!(Pattern::new("abc*def")
            .unwrap()
            .matches_with("abc/def", &options_not_require_literal));
        assert!(Pattern::new("abc[/]def")
            .unwrap()
            .matches_with("abc/def", &options_not_require_literal));
    }

    #[test]
    fn test_glob_pattern_matches_require_literal_leading_dot() {
        let options_require_literal_leading_dot = MatchOptions {
            case_sensitive: true,
            require_literal_separator: false,
            require_literal_leading_dot: true,
            follow_links: false,
        };
        let options_not_require_literal_leading_dot = MatchOptions {
            case_sensitive: true,
            require_literal_separator: false,
            require_literal_leading_dot: false,
            follow_links: false,
        };

        let f = |options| {
            Pattern::new("*.txt")
                .unwrap()
                .matches_with(".hello.txt", options)
        };
        assert!(f(&options_not_require_literal_leading_dot));
        assert!(!f(&options_require_literal_leading_dot));

        let f = |options| {
            Pattern::new(".*.*")
                .unwrap()
                .matches_with(".hello.txt", options)
        };
        assert!(f(&options_not_require_literal_leading_dot));
        assert!(f(&options_require_literal_leading_dot));

        let f = |options| {
            Pattern::new("aaa/bbb/*")
                .unwrap()
                .matches_with("aaa/bbb/.ccc", options)
        };
        assert!(f(&options_not_require_literal_leading_dot));
        assert!(!f(&options_require_literal_leading_dot));

        let f = |options| {
            Pattern::new("aaa/bbb/*")
                .unwrap()
                .matches_with("aaa/bbb/c.c.c.", options)
        };
        assert!(f(&options_not_require_literal_leading_dot));
        assert!(f(&options_require_literal_leading_dot));

        let f = |options| {
            Pattern::new("aaa/bbb/.*")
                .unwrap()
                .matches_with("aaa/bbb/.ccc", options)
        };
        assert!(f(&options_not_require_literal_leading_dot));
        assert!(f(&options_require_literal_leading_dot));

        let f = |options| {
            Pattern::new("aaa/?bbb")
                .unwrap()
                .matches_with("aaa/.bbb", options)
        };
        assert!(f(&options_not_require_literal_leading_dot));
        assert!(!f(&options_require_literal_leading_dot));

        let f = |options| {
            Pattern::new("aaa/[.]bbb")
                .unwrap()
                .matches_with("aaa/.bbb", options)
        };
        assert!(f(&options_not_require_literal_leading_dot));
        assert!(!f(&options_require_literal_leading_dot));

        let f = |options| Pattern::new("**/*").unwrap().matches_with(".bbb", options);
        assert!(f(&options_not_require_literal_leading_dot));
        assert!(!f(&options_require_literal_leading_dot));
    }

    #[test]
    fn test_glob_matches_path() {
        // on windows, (Path::new("a/b").as_str().unwrap() == "a\\b"), so this
        // tests that / and \ are considered equivalent on windows
        assert!(Pattern::new("a/b").unwrap().matches_path(&Path::new("a/b")));
    }

    #[test]
    fn test_glob_path_join() {
        let pattern = Path::new("one").join(&Path::new("**/*.rs"));
        assert!(Pattern::new(pattern.to_str().unwrap()).is_ok());
    }

    #[test]
    fn test_glob_wildcard_jpg() {
        let mut cnt = 0;
        for entry in glob(&mk_filesmap(), "/*.jpg").unwrap() {
            match entry {
                Ok(path) => {
                    cnt += 1;
                    println!("{:?}", path.display())
                }

                // if the path matched but was unreadable,
                // thereby preventing its contents from matching
                Err(e) => println!("{:?}", e),
            }
        }
        assert_eq!(cnt, 2);
    }

    #[test]
    fn test_glob_wildcard_middle_jpg() {
        let mut cnt = 0;
        for entry in glob(&mk_filesmap(), "/k*s.jpg").unwrap() {
            match entry {
                Ok(path) => {
                    cnt += 1;
                    println!("{:?}", path.display())
                }

                // if the path matched but was unreadable,
                // thereby preventing its contents from matching
                Err(e) => println!("{:?}", e),
            }
        }
        assert_eq!(cnt, 1);
    }

    #[test]
    fn test_glob_wildcard_trailing_jpg() {
        let mut cnt = 0;
        for entry in glob(&mk_filesmap(), "/k*").unwrap() {
            match entry {
                Ok(path) => {
                    cnt += 1;
                    println!("{:?}", path.display())
                }

                // if the path matched but was unreadable,
                // thereby preventing its contents from matching
                Err(e) => println!("{:?}", e),
            }
        }
        assert_eq!(cnt, 1);
    }

    #[test]
    fn test_glob_range_jpg() {
        let mut cnt = 0;
        for entry in glob(&mk_filesmap(), "/sub/?[0-2].*").unwrap() {
            match entry {
                Ok(path) => {
                    cnt += 1;
                    println!("{:?}", path.display())
                }

                // if the path matched but was unreadable,
                // thereby preventing its contents from matching
                Err(e) => println!("{:?}", e),
            }
        }
        assert_eq!(cnt, 2);
    }
}