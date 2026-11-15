//! glint_rt: the runtime of the code Glint generates for rasn.
//!
//! Vendor this file next to the generated modules; it needs only the `rasn` crate. It interprets the constraint
//! programs generated types carry, resolves open types through the dispatch enums generated from object sets,
//! decodes and encodes the contents of `CONTAINING` octet and bit strings, and shows any value as a normalized tree.
//! Decode with [`decode`] (BER) or [`decode_der`], and encode with [`encode`]: they resolve every open type and
//! check every constraint, so a value read or written through them is valid.
//!
//! SPDX-License-Identifier: MIT-0
//!
//! Permission is hereby granted, free of charge, to any person obtaining a copy of this software and associated
//! documentation files (the "Software"), to deal in the Software without restriction, including without limitation
//! the rights to use, copy, modify, merge, publish, distribute, sublicense, and/or sell copies of the Software, and to
//! permit persons to whom the Software is furnished to do so.
//!
//! THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO
//! THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
//! AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT,
//! TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
//! SOFTWARE.

#![allow(dead_code, clippy::all, clippy::pedantic)]

use std::fmt;

use rasn::types::{
    Any, BitString, BmpString, Class, Constraints, GeneralString, GeneralizedTime, GraphicString,
    Ia5String, Identifier, Integer, NumericString, ObjectIdentifier, OctetString, PrintableString,
    SetOf, Tag, TeletexString, UtcTime, Utf8String, VisibleString,
};
use rasn::{AsnType, Decode, Decoder, Encode, Encoder};

/// The version of this file; generated code names the version it was written for.
pub const VERSION: &str = "1";

// ---- errors -------------------------------------------------------------------------------------

/// Why a value could not be decoded, resolved, checked or encoded, and where.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    /// The ASN.1 path of the value: `tbsCertificate.extensions[3].extnValue`.
    pub path: String,
    pub message: String,
}

impl Error {
    pub fn new(path: &str, message: impl Into<String>) -> Self {
        Self {
            path: path.to_owned(),
            message: message.into(),
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.path.is_empty() {
            f.write_str(&self.message)
        } else {
            write!(f, "{}: {}", self.path, self.message)
        }
    }
}

impl std::error::Error for Error {}

/// The path of a component.
pub fn join(path: &str, name: &str) -> String {
    if path.is_empty() {
        name.to_owned()
    } else {
        format!("{path}.{name}")
    }
}

/// The path of an element.
pub fn index(path: &str, i: usize) -> String {
    format!("{path}[{i}]")
}

// ---- entry points -------------------------------------------------------------------------------

fn finish<T: Glint>(value: Result<(T, &[u8]), rasn::error::DecodeError>) -> Result<T, Error> {
    let (mut value, rest) = value.map_err(|e| Error::new("", e.to_string()))?;
    if !rest.is_empty() {
        return Err(Error::new("", format!("{} octets follow the value", rest.len())));
    }
    value.resolve("")?;
    value.validate("")?;
    Ok(value)
}

/// Decodes a BER encoding, resolves its open types and checks its constraints.
pub fn decode<T: Decode + Glint>(bytes: &[u8]) -> Result<T, Error> {
    finish(rasn::ber::decode_with_remainder::<T>(bytes))
}

/// Decodes a DER encoding, resolves its open types and checks its constraints.
pub fn decode_der<T: Decode + Glint>(bytes: &[u8]) -> Result<T, Error> {
    finish(rasn::der::decode_with_remainder::<T>(bytes))
}

/// Checks a value's open types against their keys and its constraints, then encodes it in DER.
pub fn encode<T: Encode + Glint + Clone>(value: &T) -> Result<Vec<u8>, Error> {
    let mut resolved = value.clone();
    resolved.resolve("")?;
    resolved.validate("")?;
    rasn::der::encode(&resolved).map_err(|e| Error::new("", e.to_string()))
}

/// Decodes the contents of a contained value or an open type, and resolves its open types.
pub fn decode_inner<T: Decode + Glint>(bytes: &[u8], path: &str) -> Result<T, Error> {
    let (mut value, rest) =
        rasn::ber::decode_with_remainder::<T>(bytes).map_err(|e| Error::new(path, e.to_string()))?;
    if !rest.is_empty() {
        return Err(Error::new(path, format!("{} octets follow the contained value", rest.len())));
    }
    value.resolve(path)?;
    Ok(value)
}

// ---- the value tree -----------------------------------------------------------------------------

/// A value shown the way the decoder shows it, for comparison and display: a leaf's text, a constructed value's
/// components or elements, a contained value, or a raw TLV.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tree {
    /// The component (`tbsCertificate`), or the chosen alternatives (`directoryName.rdnSequence`).
    pub label: Option<String>,
    pub body: Body,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Body {
    Leaf(String),
    List(Vec<Tree>),
    Contains(Box<Tree>),
    /// The TLV in hexadecimal.
    Raw(String),
}

impl Tree {
    pub fn leaf(text: impl Into<String>) -> Self {
        Self {
            label: None,
            body: Body::Leaf(text.into()),
        }
    }

    pub fn list(items: Vec<Tree>) -> Self {
        Self {
            label: None,
            body: Body::List(items),
        }
    }

    /// The tree under `name`: `name`, or `name.label` when it already has one (a chosen alternative).
    pub fn labeled(mut self, name: &str) -> Self {
        self.label = Some(match self.label.take() {
            Some(inner) => format!("{name}.{inner}"),
            None => name.to_owned(),
        });
        self
    }

    pub fn to_json(&self) -> String {
        let mut out = String::new();
        self.write_json(&mut out);
        out
    }

    fn write_json(&self, out: &mut String) {
        out.push('{');
        if let Some(label) = &self.label {
            out.push_str("\"label\":");
            json_string(label, out);
            out.push(',');
        }
        match &self.body {
            Body::Leaf(text) => {
                out.push_str("\"leaf\":");
                json_string(text, out);
            }
            Body::Raw(hex) => {
                out.push_str("\"raw\":");
                json_string(hex, out);
            }
            Body::Contains(inner) => {
                out.push_str("\"contains\":");
                inner.write_json(out);
            }
            Body::List(items) => {
                out.push_str("\"list\":[");
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    item.write_json(out);
                }
                out.push(']');
            }
        }
        out.push('}');
    }
}

fn json_string(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02X}")).collect()
}

/// A list's tree, each element under `name` when the list names its elements.
pub fn list_tree<'a>(items: impl Iterator<Item = Tree>, name: Option<&str>) -> Tree {
    Tree::list(
        items
            .map(|t| match name {
                Some(name) => t.labeled(name),
                None => t,
            })
            .collect(),
    )
}

// ---- keys ---------------------------------------------------------------------------------------

/// The value of the unique field of an object: what selects an open type's type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Key {
    Oid(Vec<u32>),
    Int(i128),
}

impl Key {
    pub fn of(value: &dyn Glint) -> Option<Key> {
        value
            .oid()
            .map(|arcs| Key::Oid(arcs.to_vec()))
            .or_else(|| value.int().map(Key::Int))
    }

    pub fn is_oid(&self, arcs: &[u32]) -> bool {
        matches!(self, Key::Oid(own) if own.as_slice() == arcs)
    }

    pub fn is_int(&self, n: i128) -> bool {
        matches!(self, Key::Int(own) if *own == n)
    }
}

impl fmt::Display for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Key::Oid(arcs) => {
                let parts: Vec<String> = arcs.iter().map(u32::to_string).collect();
                f.write_str(&parts.join("."))
            }
            Key::Int(n) => write!(f, "{n}"),
        }
    }
}

/// An open type's dispatch: the generated enum of the types an object set's field gives its keys.
pub trait Open: Sized {
    /// Decodes `bytes` as the type `key` selects: `None` when the set gives the key no type (the value stays raw),
    /// an error when the set is not extensible and has no object with the key.
    fn open(key: Option<&Key>, bytes: &[u8], path: &str) -> Result<Option<Self>, Error>;
    /// The key of the variant, `None` for a raw value.
    fn key(&self) -> Option<Key>;
}

/// Resolves a dispatch enum's raw value with `key`, or checks that its variant has that key.
pub fn dispatch_open<T: Open>(value: &mut T, raw: Option<&[u8]>, key: Option<&Key>, path: &str) -> Result<(), Error> {
    match raw {
        Some(bytes) => {
            if let Some(resolved) = T::open(key, bytes, path)? {
                *value = resolved;
            }
            Ok(())
        }
        None => match (key, value.key()) {
            (Some(key), Some(own)) if *key != own => Err(Error::new(
                path,
                format!("the value is the type of the object keyed by {own}, but the key is {key}"),
            )),
            _ => Ok(()),
        },
    }
}

/// An open type a record's components key, by component paths from the record (X.682 10.7).
pub struct Resolution {
    pub key: &'static [&'static str],
    pub open: &'static [&'static str],
    pub text: &'static str,
}

/// The value at a path of components, through aliases; `None` when a component is absent.
pub fn lookup<'a>(value: &'a dyn Glint, path: &[&str]) -> Option<&'a dyn Glint> {
    match path.split_first() {
        None => Some(value),
        Some((first, rest)) => lookup(value.component(first)??, rest),
    }
}

fn navigate(value: &mut dyn Glint, path: &[&str], key: Option<&Key>, at: &str) -> Result<(), Error> {
    let Some((first, rest)) = path.split_first() else {
        return value.dispatch(key, at);
    };
    let mut result = Ok(());
    let at_list = value.each_mut(&mut |element, i| {
        if result.is_ok() {
            result = navigate(element, path, key, &index(at, i));
        }
    });
    if at_list {
        return result;
    }
    match value.component_mut(first) {
        Some(Some(component)) => navigate(component, rest, key, &join(at, first)),
        _ => Ok(()),
    }
}

/// Resolves one open type below `owner` with the key its relation names.
pub fn resolve_at(owner: &mut dyn Glint, resolution: &Resolution, path: &str) -> Result<(), Error> {
    let key = lookup(&*owner, resolution.key).and_then(Key::of);
    navigate(owner, resolution.open, key.as_ref(), path)
}

// ---- constraint programs ------------------------------------------------------------------------

/// A check over one value: a constraint, as the generated code carries it.
#[derive(Debug, Clone, Copy)]
pub enum Check {
    Pass,
    All(&'static [Check]),
    Any(&'static [Check]),
    Not(&'static Check),
    /// The integer (a value, a size or a code point) is in one of the ranges.
    Int(&'static [(Option<i128>, Option<i128>)]),
    Equals(Const),
    Size(&'static Check),
    Alphabet(&'static Check),
    /// The component, when present, passes.
    Component(&'static str, &'static Check),
    /// The component or alternative is present (`true`) or absent.
    Presence(&'static str, bool),
    Each(&'static Check),
}

/// A value to compare with.
#[derive(Debug, Clone, Copy)]
pub enum Const {
    Bool(bool),
    Int(i128),
    Null,
    Oid(&'static [u32]),
    Str(&'static str),
    Bits(&'static [bool]),
    Octets(&'static [u8]),
}

/// A constraint and its text, for messages.
#[derive(Debug, Clone, Copy)]
pub struct Rule {
    pub check: Check,
    pub text: &'static str,
}

fn int_passes(check: &Check, n: i128) -> bool {
    match check {
        Check::Pass => true,
        Check::All(items) => items.iter().all(|c| int_passes(c, n)),
        Check::Any(items) => items.iter().any(|c| int_passes(c, n)),
        Check::Not(c) => !int_passes(c, n),
        Check::Int(ranges) => ranges
            .iter()
            .any(|(lo, hi)| lo.map_or(true, |lo| n >= lo) && hi.map_or(true, |hi| n <= hi)),
        Check::Equals(Const::Int(v)) => n == *v,
        _ => true,
    }
}

/// Whether `value` passes `check`; what a value's type cannot answer passes.
pub fn passes(check: &Check, value: &dyn Glint) -> bool {
    match check {
        Check::Pass => true,
        Check::All(items) => items.iter().all(|c| passes(c, value)),
        Check::Any(items) => items.iter().any(|c| passes(c, value)),
        Check::Not(c) => !passes(c, value),
        Check::Int(_) => value.int().map_or(true, |n| int_passes(check, n)),
        Check::Equals(c) => value.equals(c).unwrap_or(true),
        Check::Size(c) => value
            .size()
            .map_or(true, |n| int_passes(c, i128::try_from(n).unwrap_or(i128::MAX))),
        Check::Alphabet(c) => value
            .chars()
            .map_or(true, |chars| chars.iter().all(|&ch| int_passes(c, i128::from(ch)))),
        Check::Component(name, c) => match value.component(name) {
            Some(Some(component)) => passes(c, component),
            _ => true,
        },
        Check::Presence(name, present) => match value.component(name) {
            Some(component) => component.is_some() == *present,
            None => true,
        },
        Check::Each(c) => {
            let mut ok = true;
            value.each(&mut |element, _| ok = ok && passes(c, element));
            ok
        }
    }
}

/// Checks `value` against every rule.
pub fn check(rules: &[Rule], value: &dyn Glint, path: &str) -> Result<(), Error> {
    for rule in rules {
        if !passes(&rule.check, value) {
            return Err(Error::new(path, format!("the value is outside the constraint {}", rule.text)));
        }
    }
    Ok(())
}

// ---- the trait generated types implement --------------------------------------------------------

/// What generated types (and the library types they use) answer: checks, resolution, inspection and the tree.
pub trait Glint {
    /// Checks the constraints of the value and of everything in it.
    fn validate(&self, path: &str) -> Result<(), Error> {
        let _ = path;
        Ok(())
    }

    /// Resolves the open types in the value.
    fn resolve(&mut self, path: &str) -> Result<(), Error> {
        let _ = path;
        Ok(())
    }

    fn tree(&self) -> Tree;

    /// An integer or enumerated value.
    fn int(&self) -> Option<i128> {
        None
    }

    /// The size the `SIZE` constraint measures: characters, bits, octets or elements.
    fn size(&self) -> Option<usize> {
        None
    }

    /// The code points of a character string.
    fn chars(&self) -> Option<Vec<u32>> {
        None
    }

    fn oid(&self) -> Option<&[u32]> {
        None
    }

    /// Whether the value equals `c`; `None` when the type cannot tell.
    fn equals(&self, c: &Const) -> Option<bool> {
        let _ = c;
        None
    }

    /// A record's component or a choice's alternative by ASN.1 name: `Some(None)` when it is absent, `None` when the
    /// value has no such component.
    fn component(&self, name: &str) -> Option<Option<&dyn Glint>> {
        let _ = name;
        None
    }

    fn component_mut(&mut self, name: &str) -> Option<Option<&mut dyn Glint>> {
        let _ = name;
        None
    }

    /// Calls `f` with each element of a list; `false` when the value is not a list.
    fn each(&self, f: &mut dyn FnMut(&dyn Glint, usize)) -> bool {
        let _ = f;
        false
    }

    fn each_mut(&mut self, f: &mut dyn FnMut(&mut dyn Glint, usize)) -> bool {
        let _ = f;
        false
    }

    /// Resolves an open type with its key, or checks that its value has that key.
    fn dispatch(&mut self, key: Option<&Key>, path: &str) -> Result<(), Error> {
        let _ = (key, path);
        Ok(())
    }
}

impl<T: Glint + ?Sized> Glint for Box<T> {
    fn validate(&self, path: &str) -> Result<(), Error> {
        (**self).validate(path)
    }
    fn resolve(&mut self, path: &str) -> Result<(), Error> {
        (**self).resolve(path)
    }
    fn tree(&self) -> Tree {
        (**self).tree()
    }
    fn int(&self) -> Option<i128> {
        (**self).int()
    }
    fn size(&self) -> Option<usize> {
        (**self).size()
    }
    fn chars(&self) -> Option<Vec<u32>> {
        (**self).chars()
    }
    fn oid(&self) -> Option<&[u32]> {
        (**self).oid()
    }
    fn equals(&self, c: &Const) -> Option<bool> {
        (**self).equals(c)
    }
    fn component(&self, name: &str) -> Option<Option<&dyn Glint>> {
        (**self).component(name)
    }
    fn component_mut(&mut self, name: &str) -> Option<Option<&mut dyn Glint>> {
        (**self).component_mut(name)
    }
    fn each(&self, f: &mut dyn FnMut(&dyn Glint, usize)) -> bool {
        (**self).each(f)
    }
    fn each_mut(&mut self, f: &mut dyn FnMut(&mut dyn Glint, usize)) -> bool {
        (**self).each_mut(f)
    }
    fn dispatch(&mut self, key: Option<&Key>, path: &str) -> Result<(), Error> {
        (**self).dispatch(key, path)
    }
}

impl<T: Glint> Glint for Vec<T> {
    fn validate(&self, path: &str) -> Result<(), Error> {
        self.iter()
            .enumerate()
            .try_for_each(|(i, e)| e.validate(&index(path, i)))
    }
    fn resolve(&mut self, path: &str) -> Result<(), Error> {
        self.iter_mut()
            .enumerate()
            .try_for_each(|(i, e)| e.resolve(&index(path, i)))
    }
    fn tree(&self) -> Tree {
        list_tree(self.iter().map(Glint::tree), None)
    }
    fn size(&self) -> Option<usize> {
        Some(self.len())
    }
    fn each(&self, f: &mut dyn FnMut(&dyn Glint, usize)) -> bool {
        for (i, e) in self.iter().enumerate() {
            f(e, i);
        }
        true
    }
    fn each_mut(&mut self, f: &mut dyn FnMut(&mut dyn Glint, usize)) -> bool {
        for (i, e) in self.iter_mut().enumerate() {
            f(e, i);
        }
        true
    }
    fn dispatch(&mut self, key: Option<&Key>, path: &str) -> Result<(), Error> {
        self.iter_mut()
            .enumerate()
            .try_for_each(|(i, e)| e.dispatch(key, &index(path, i)))
    }
}

/// Applies `f` to every element of a `SET OF`, which keeps its elements behind its own API.
fn set_of_mut<T: Eq + Clone>(set: &mut SetOf<T>, f: &mut dyn FnMut(&mut T, usize)) {
    let mut elements: Vec<T> = set.to_vec().into_iter().cloned().collect();
    for (i, e) in elements.iter_mut().enumerate() {
        f(e, i);
    }
    *set = SetOf::from_vec(elements);
}

impl<T: Glint + Eq + Clone> Glint for SetOf<T> {
    fn validate(&self, path: &str) -> Result<(), Error> {
        self.iter()
            .enumerate()
            .try_for_each(|(i, e)| e.validate(&index(path, i)))
    }
    fn resolve(&mut self, path: &str) -> Result<(), Error> {
        let mut result = Ok(());
        set_of_mut(self, &mut |e, i| {
            if result.is_ok() {
                result = e.resolve(&index(path, i));
            }
        });
        result
    }
    fn tree(&self) -> Tree {
        list_tree(self.iter().map(Glint::tree), None)
    }
    fn size(&self) -> Option<usize> {
        Some(self.len())
    }
    fn each(&self, f: &mut dyn FnMut(&dyn Glint, usize)) -> bool {
        for (i, e) in self.iter().enumerate() {
            f(e, i);
        }
        true
    }
    fn each_mut(&mut self, f: &mut dyn FnMut(&mut dyn Glint, usize)) -> bool {
        set_of_mut(self, &mut |e, i| f(e, i));
        true
    }
    fn dispatch(&mut self, key: Option<&Key>, path: &str) -> Result<(), Error> {
        let mut result = Ok(());
        set_of_mut(self, &mut |e, i| {
            if result.is_ok() {
                result = e.dispatch(key, &index(path, i));
            }
        });
        result
    }
}

// ---- library types ------------------------------------------------------------------------------

/// The contents octets of a DER encoding: what the decoder renders.
fn contents_of(encoding: &[u8]) -> &[u8] {
    let mut i = 1;
    if encoding.first().is_some_and(|b| b & 0x1f == 0x1f) {
        while encoding.get(i).is_some_and(|b| b & 0x80 != 0) {
            i += 1;
        }
        i += 1;
    }
    let Some(&first) = encoding.get(i) else {
        return &[];
    };
    i += 1;
    if first & 0x80 != 0 {
        i += usize::from(first & 0x7f);
    }
    encoding.get(i..).unwrap_or(&[])
}

fn der_contents<T: Encode>(value: &T) -> Vec<u8> {
    rasn::der::encode(value)
        .map(|e| contents_of(&e).to_vec())
        .unwrap_or_default()
}

/// How a character string's contents are read.
#[derive(Clone, Copy)]
enum Text {
    Utf8,
    Ucs2,
    Ucs4,
    Latin1,
}

fn text_of(contents: &[u8], how: Text) -> String {
    match how {
        Text::Utf8 => String::from_utf8_lossy(contents).into_owned(),
        Text::Ucs2 => char::decode_utf16(contents.chunks_exact(2).map(|c| u16::from_be_bytes([c[0], c[1]])))
            .map(|c| c.unwrap_or(char::REPLACEMENT_CHARACTER))
            .collect(),
        Text::Ucs4 => contents
            .chunks_exact(4)
            .map(|c| char::from_u32(u32::from_be_bytes([c[0], c[1], c[2], c[3]])).unwrap_or(char::REPLACEMENT_CHARACTER))
            .collect(),
        Text::Latin1 => contents.iter().map(|&b| char::from(b)).collect(),
    }
}

impl Glint for bool {
    fn tree(&self) -> Tree {
        Tree::leaf(if *self { "true" } else { "false" })
    }
    fn equals(&self, c: &Const) -> Option<bool> {
        match c {
            Const::Bool(v) => Some(v == self),
            _ => None,
        }
    }
}

impl Glint for () {
    fn tree(&self) -> Tree {
        Tree::leaf("NULL")
    }
    fn equals(&self, c: &Const) -> Option<bool> {
        Some(matches!(c, Const::Null))
    }
}

macro_rules! native_integers {
    ($($t:ty),*) => {$(
        impl Glint for $t {
            fn tree(&self) -> Tree {
                Tree::leaf(self.to_string())
            }
            fn int(&self) -> Option<i128> {
                i128::try_from(*self).ok()
            }
            fn equals(&self, c: &Const) -> Option<bool> {
                match c {
                    Const::Int(v) => Some(i128::try_from(*self).ok() == Some(*v)),
                    _ => None,
                }
            }
        }
    )*};
}

native_integers!(u8, u16, u32, u64, i8, i16, i32, i64);

impl Glint for Integer {
    fn tree(&self) -> Tree {
        Tree::leaf(self.to_string())
    }
    fn int(&self) -> Option<i128> {
        i128::try_from(self).ok()
    }
    fn equals(&self, c: &Const) -> Option<bool> {
        match c {
            Const::Int(v) => Some(i128::try_from(self).ok() == Some(*v)),
            _ => None,
        }
    }
}

impl Glint for OctetString {
    fn tree(&self) -> Tree {
        Tree::leaf(hex(self))
    }
    fn size(&self) -> Option<usize> {
        Some(self.len())
    }
    fn equals(&self, c: &Const) -> Option<bool> {
        match c {
            Const::Octets(v) => Some(self.as_ref() == *v),
            _ => None,
        }
    }
}

impl Glint for BitString {
    fn tree(&self) -> Tree {
        let contents = der_contents(self);
        match contents.split_first() {
            Some((unused, bits)) => Tree::leaf(format!("{unused}:{}", hex(bits))),
            None => Tree::leaf(""),
        }
    }
    fn size(&self) -> Option<usize> {
        Some(self.len())
    }
    fn equals(&self, c: &Const) -> Option<bool> {
        match c {
            Const::Bits(v) => {
                let own: Vec<bool> = self.iter().map(|b| *b).collect();
                let trim = |b: &[bool]| b.iter().rposition(|&x| x).map_or(0, |p| p + 1);
                Some(own[..trim(&own)] == v[..trim(v)])
            }
            _ => None,
        }
    }
}

/// A bit string of `bits`.
pub fn bits(bits: &[bool]) -> BitString {
    bits.iter().copied().collect()
}

/// An object identifier of `arcs`.
pub fn oid(arcs: &'static [u32]) -> ObjectIdentifier {
    ObjectIdentifier::new_unchecked(std::borrow::Cow::Borrowed(arcs))
}

impl Glint for ObjectIdentifier {
    fn tree(&self) -> Tree {
        Tree::leaf(Key::Oid(self.to_vec()).to_string())
    }
    fn oid(&self) -> Option<&[u32]> {
        Some(self)
    }
    fn equals(&self, c: &Const) -> Option<bool> {
        match c {
            Const::Oid(v) => Some(&self[..] == *v),
            _ => None,
        }
    }
}

impl Glint for Any {
    fn tree(&self) -> Tree {
        Tree {
            label: None,
            body: Body::Raw(hex(self.as_bytes())),
        }
    }
}

macro_rules! strings {
    ($($t:ty => $how:expr),* $(,)?) => {$(
        impl Glint for $t {
            fn tree(&self) -> Tree {
                Tree::leaf(text_of(&der_contents(self), $how))
            }
            fn size(&self) -> Option<usize> {
                Some(text_of(&der_contents(self), $how).as_str().chars().count())
            }
            fn chars(&self) -> Option<Vec<u32>> {
                Some(text_of(&der_contents(self), $how).as_str().chars().map(u32::from).collect())
            }
            fn equals(&self, c: &Const) -> Option<bool> {
                match c {
                    Const::Str(v) => Some(text_of(&der_contents(self), $how) == *v),
                    _ => None,
                }
            }
        }
    )*};
}

strings!(
    Utf8String => Text::Utf8,
    PrintableString => Text::Latin1,
    Ia5String => Text::Latin1,
    VisibleString => Text::Latin1,
    NumericString => Text::Latin1,
    TeletexString => Text::Latin1,
    GeneralString => Text::Latin1,
    GraphicString => Text::Latin1,
    BmpString => Text::Ucs2,
);

macro_rules! times {
    ($($t:ty),*) => {$(
        impl Glint for $t {
            fn tree(&self) -> Tree {
                Tree::leaf(text_of(&der_contents(self), Text::Latin1))
            }
        }
    )*};
}

times!(UtcTime, GeneralizedTime);

/// A primitive universal type rasn has no type for (`UniversalString`, `VideotexString`, `ObjectDescriptor`,
/// `RELATIVE-OID`, `DATE`, `TIME-OF-DAY`, `DATE-TIME`, `DURATION`, `TIME`): its contents octets, as they are.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct Raw<const TAG: u32>(pub Vec<u8>);

impl<const TAG: u32> AsnType for Raw<TAG> {
    const TAG: Tag = Tag::new(Class::Universal, TAG);
}

impl<const TAG: u32> Decode for Raw<TAG> {
    fn decode_with_tag_and_constraints<D: Decoder>(decoder: &mut D, tag: Tag, constraints: &Constraints) -> Result<Self, D::Error> {
        decoder.decode_octet_string::<Vec<u8>>(tag, constraints).map(Self)
    }
}

impl<const TAG: u32> Encode for Raw<TAG> {
    fn encode_with_tag_and_constraints<'b, E: Encoder<'b>>(
        &self,
        encoder: &mut E,
        tag: Tag,
        constraints: &Constraints,
        identifier: Identifier,
    ) -> Result<(), E::Error> {
        encoder.encode_octet_string(tag, constraints, &self.0, identifier).map(drop)
    }
}

impl<const TAG: u32> Glint for Raw<TAG> {
    fn tree(&self) -> Tree {
        Tree::leaf(match TAG {
            28 => text_of(&self.0, Text::Ucs4),
            13 => relative_oid(&self.0),
            _ => text_of(&self.0, Text::Latin1),
        })
    }
    fn size(&self) -> Option<usize> {
        match TAG {
            28 => Some(self.0.len() / 4),
            13 => None,
            _ => Some(self.0.len()),
        }
    }
    fn chars(&self) -> Option<Vec<u32>> {
        match TAG {
            28 => Some(text_of(&self.0, Text::Ucs4).as_str().chars().map(u32::from).collect()),
            13 => None,
            _ => Some(self.0.iter().map(|&b| u32::from(b)).collect()),
        }
    }
}

fn relative_oid(contents: &[u8]) -> String {
    let mut arcs = Vec::new();
    let mut arc: u64 = 0;
    for &b in contents {
        arc = arc << 7 | u64::from(b & 0x7f);
        if b & 0x80 == 0 {
            arcs.push(arc.to_string());
            arc = 0;
        }
    }
    arcs.join(".")
}

// ---- contents constraints -----------------------------------------------------------------------

fn decode_error<D: Decoder>(e: Error) -> D::Error {
    <D::Error as rasn::de::Error>::custom(e, rasn::Codec::Ber)
}

fn encode_error<'b, E: Encoder<'b>>(e: impl fmt::Display) -> E::Error {
    <E::Error as rasn::enc::Error>::custom(e, rasn::Codec::Der)
}

/// `OCTET STRING (CONTAINING T)`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Containing<T>(pub T);

impl<T> AsnType for Containing<T> {
    const TAG: Tag = Tag::OCTET_STRING;
}

impl<T: Decode + Glint> Decode for Containing<T> {
    fn decode_with_tag_and_constraints<D: Decoder>(decoder: &mut D, tag: Tag, constraints: &Constraints) -> Result<Self, D::Error> {
        let octets = decoder.decode_octet_string::<Vec<u8>>(tag, constraints)?;
        decode_inner(&octets, "").map(Self).map_err(decode_error::<D>)
    }
}

impl<T: Encode> Encode for Containing<T> {
    fn encode_with_tag_and_constraints<'b, E: Encoder<'b>>(
        &self,
        encoder: &mut E,
        tag: Tag,
        constraints: &Constraints,
        identifier: Identifier,
    ) -> Result<(), E::Error> {
        let octets = rasn::der::encode(&self.0).map_err(encode_error::<E>)?;
        encoder.encode_octet_string(tag, constraints, &octets, identifier).map(drop)
    }
}

impl<T: Glint> Glint for Containing<T> {
    fn validate(&self, path: &str) -> Result<(), Error> {
        self.0.validate(path)
    }
    fn resolve(&mut self, path: &str) -> Result<(), Error> {
        self.0.resolve(path)
    }
    fn tree(&self) -> Tree {
        Tree {
            label: None,
            body: Body::Contains(Box::new(self.0.tree())),
        }
    }
    fn component(&self, name: &str) -> Option<Option<&dyn Glint>> {
        self.0.component(name)
    }
    fn component_mut(&mut self, name: &str) -> Option<Option<&mut dyn Glint>> {
        self.0.component_mut(name)
    }
}

/// `BIT STRING (CONTAINING T)`: the encoding fills whole octets.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct BitsContaining<T>(pub T);

impl<T> AsnType for BitsContaining<T> {
    const TAG: Tag = Tag::BIT_STRING;
}

fn whole_octets(bits: &BitString) -> Option<Vec<u8>> {
    (bits.len() % 8 == 0).then(|| bits.as_raw_slice().to_vec())
}

impl<T: Decode + Glint> Decode for BitsContaining<T> {
    fn decode_with_tag_and_constraints<D: Decoder>(decoder: &mut D, tag: Tag, constraints: &Constraints) -> Result<Self, D::Error> {
        let bits = decoder.decode_bit_string(tag, constraints)?;
        let octets = whole_octets(&bits)
            .ok_or_else(|| decode_error::<D>(Error::new("", "the contained value does not fill whole octets")))?;
        decode_inner(&octets, "").map(Self).map_err(decode_error::<D>)
    }
}

impl<T: Encode> Encode for BitsContaining<T> {
    fn encode_with_tag_and_constraints<'b, E: Encoder<'b>>(
        &self,
        encoder: &mut E,
        tag: Tag,
        constraints: &Constraints,
        identifier: Identifier,
    ) -> Result<(), E::Error> {
        let octets = rasn::der::encode(&self.0).map_err(encode_error::<E>)?;
        let bits = BitString::from_vec(octets);
        encoder.encode_bit_string(tag, constraints, &bits, identifier).map(drop)
    }
}

impl<T: Glint> Glint for BitsContaining<T> {
    fn validate(&self, path: &str) -> Result<(), Error> {
        self.0.validate(path)
    }
    fn resolve(&mut self, path: &str) -> Result<(), Error> {
        self.0.resolve(path)
    }
    fn tree(&self) -> Tree {
        Tree {
            label: None,
            body: Body::Contains(Box::new(self.0.tree())),
        }
    }
}

/// The value of a string whose contents are an open type: decoded once its key is known, or the octets as they
/// are when the object with the key leaves the type out.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Contained<T, R> {
    Value(T),
    Raw(R),
}

/// `OCTET STRING (CONTAINING CLASS.&Field({Set}{@key}))`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ContainingOpen<T>(pub Contained<T, OctetString>);

impl<T> AsnType for ContainingOpen<T> {
    const TAG: Tag = Tag::OCTET_STRING;
}

impl<T> Decode for ContainingOpen<T> {
    fn decode_with_tag_and_constraints<D: Decoder>(decoder: &mut D, tag: Tag, constraints: &Constraints) -> Result<Self, D::Error> {
        let octets = decoder.decode_octet_string::<Vec<u8>>(tag, constraints)?;
        Ok(Self(Contained::Raw(OctetString::from(octets))))
    }
}

impl<T: Encode> Encode for ContainingOpen<T> {
    fn encode_with_tag_and_constraints<'b, E: Encoder<'b>>(
        &self,
        encoder: &mut E,
        tag: Tag,
        constraints: &Constraints,
        identifier: Identifier,
    ) -> Result<(), E::Error> {
        let octets = match &self.0 {
            Contained::Value(v) => rasn::der::encode(v).map_err(encode_error::<E>)?,
            Contained::Raw(raw) => raw.to_vec(),
        };
        encoder.encode_octet_string(tag, constraints, &octets, identifier).map(drop)
    }
}

impl<T: Glint + Open> Glint for ContainingOpen<T> {
    fn validate(&self, path: &str) -> Result<(), Error> {
        match &self.0 {
            Contained::Value(v) => v.validate(path),
            Contained::Raw(_) => Ok(()),
        }
    }
    fn resolve(&mut self, path: &str) -> Result<(), Error> {
        match &mut self.0 {
            Contained::Value(v) => v.resolve(path),
            Contained::Raw(_) => Ok(()),
        }
    }
    fn tree(&self) -> Tree {
        match &self.0 {
            Contained::Value(v) => Tree {
                label: None,
                body: Body::Contains(Box::new(v.tree())),
            },
            Contained::Raw(raw) => raw.tree(),
        }
    }
    fn dispatch(&mut self, key: Option<&Key>, path: &str) -> Result<(), Error> {
        match &mut self.0 {
            Contained::Raw(raw) => {
                if let Some(value) = T::open(key, raw, path)? {
                    self.0 = Contained::Value(value);
                }
                Ok(())
            }
            Contained::Value(v) => v.dispatch(key, path),
        }
    }
}

/// `BIT STRING (CONTAINING CLASS.&Field({Set}{@key}))`: a signature, whose algorithm says what it holds.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct BitsContainingOpen<T>(pub Contained<T, BitString>);

impl<T> AsnType for BitsContainingOpen<T> {
    const TAG: Tag = Tag::BIT_STRING;
}

impl<T> Decode for BitsContainingOpen<T> {
    fn decode_with_tag_and_constraints<D: Decoder>(decoder: &mut D, tag: Tag, constraints: &Constraints) -> Result<Self, D::Error> {
        decoder
            .decode_bit_string(tag, constraints)
            .map(|bits| Self(Contained::Raw(bits)))
    }
}

impl<T: Encode> Encode for BitsContainingOpen<T> {
    fn encode_with_tag_and_constraints<'b, E: Encoder<'b>>(
        &self,
        encoder: &mut E,
        tag: Tag,
        constraints: &Constraints,
        identifier: Identifier,
    ) -> Result<(), E::Error> {
        let bits = match &self.0 {
            Contained::Value(v) => BitString::from_vec(rasn::der::encode(v).map_err(encode_error::<E>)?),
            Contained::Raw(raw) => raw.clone(),
        };
        encoder.encode_bit_string(tag, constraints, &bits, identifier).map(drop)
    }
}

impl<T: Glint + Open> Glint for BitsContainingOpen<T> {
    fn validate(&self, path: &str) -> Result<(), Error> {
        match &self.0 {
            Contained::Value(v) => v.validate(path),
            Contained::Raw(_) => Ok(()),
        }
    }
    fn resolve(&mut self, path: &str) -> Result<(), Error> {
        match &mut self.0 {
            Contained::Value(v) => v.resolve(path),
            Contained::Raw(_) => Ok(()),
        }
    }
    fn tree(&self) -> Tree {
        match &self.0 {
            Contained::Value(v) => Tree {
                label: None,
                body: Body::Contains(Box::new(v.tree())),
            },
            Contained::Raw(raw) => raw.tree(),
        }
    }
    fn dispatch(&mut self, key: Option<&Key>, path: &str) -> Result<(), Error> {
        match &mut self.0 {
            Contained::Raw(raw) => {
                let Some(octets) = whole_octets(raw) else {
                    return Ok(());
                };
                if let Some(value) = T::open(key, &octets, path)? {
                    self.0 = Contained::Value(value);
                }
                Ok(())
            }
            Contained::Value(v) => v.dispatch(key, path),
        }
    }
}

// ---- unknown extension additions ----------------------------------------------------------------

/// The extension additions of an extensible `SEQUENCE` that its type does not know (X.680 52): kept as they are, so
/// a value decodes (X.690 requires it) and re-encodes unchanged. Generated structs carry one as `_extensions`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct Extensions(pub Vec<Any>);

impl AsnType for Extensions {
    const TAG: Tag = Tag::EOC;
    const TAG_TREE: rasn::types::TagTree = rasn::types::TagTree::Choice(&[]);
}

impl Decode for Extensions {
    fn decode<D: Decoder>(decoder: &mut D) -> Result<Self, D::Error> {
        let mut items = Vec::new();
        while let Some(item) = Option::<Any>::decode(decoder)? {
            items.push(item);
        }
        Ok(Self(items))
    }

    fn decode_with_tag_and_constraints<D: Decoder>(decoder: &mut D, _: Tag, _: &Constraints) -> Result<Self, D::Error> {
        Self::decode(decoder)
    }
}

impl Encode for Extensions {
    fn encode<'b, E: Encoder<'b>>(&self, encoder: &mut E) -> Result<(), E::Error> {
        for item in &self.0 {
            item.encode(encoder)?;
        }
        Ok(())
    }

    fn encode_with_tag_and_constraints<'b, E: Encoder<'b>>(
        &self,
        encoder: &mut E,
        _: Tag,
        _: &Constraints,
        _: Identifier,
    ) -> Result<(), E::Error> {
        self.encode(encoder)
    }
}
