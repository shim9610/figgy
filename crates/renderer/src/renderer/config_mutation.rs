//! Leaf-mutation walker for the cache-key mutation tests (#46, tier 2).
//!
//! A cache key is complete when every input its cached computation reads is
//! in it. The tests built on this module check one direction of that: for
//! every leaf of a serialized input (a `Config`, a `SeriesConfig`), change
//! the leaf; if the cached computation's output changes and the key does
//! not, the key is missing that leaf.
//!
//! The leaves come from the serde representation, so a field added to the
//! model is walked without anyone adding a case. A `Value` alone does not
//! show fields that serialize to nothing (`skip_serializing_if`) or the
//! variants an enum could take, so the walker deserializes the document
//! through a recording [`Deserializer`] first. serde's derive hands that
//! deserializer every struct's full field list and every enum's variant
//! list, and the primitive type it expects at each path; absent fields and
//! `None` values are then given a synthesized value of that type.
//!
//! One leaf is one mutation. A leaf has an ordered list of candidate values;
//! the runner takes the first candidate the renderer's validation accepts
//! and records the leaf as skipped when none is accepted. An enum leaf runs
//! every other variant, since each variant is a different input.
//!
//! Limits: serde buffers the content of an internally tagged enum
//! (`DrawStyle`, `PickedDataRef`) before deserializing it, so the recorder
//! sees no types below one. Their present leaves are still walked by value
//! kind and their tags by the "unknown variant" list serde reports, but a
//! field that is absent there is not discovered.

use std::cell::RefCell;
use std::collections::HashMap;

use serde::Serialize;
use serde::de::{
    self, DeserializeOwned, DeserializeSeed, Deserializer, EnumAccess, IntoDeserializer, MapAccess,
    SeqAccess, VariantAccess, Visitor,
};
use serde_json::Value;

/// What serde's derive asked for at one path.
#[derive(Clone, Debug, PartialEq)]
enum Hint {
    Bool,
    Int,
    Float,
    Char,
    Str,
    Unit,
    Option,
    Newtype,
    Seq,
    Tuple(usize),
    Map,
    Struct(&'static [&'static str]),
    Enum(&'static [&'static str]),
}

type Log = RefCell<HashMap<String, Vec<Hint>>>;

/// A placeholder that no field accepts as a valid value; probing a slot with
/// it records the type the slot expects.
const SENTINEL: &str = "\u{1}figgy-probe";

fn sentinel() -> Value {
    Value::String(SENTINEL.into())
}

fn join(path: &str, key: &str) -> String {
    format!("{path}/{}", key.replace('~', "~0").replace('/', "~1"))
}

fn unescape(segment: &str) -> String {
    segment.replace("~1", "/").replace("~0", "~")
}

struct Probe<'v, 'l> {
    value: &'v Value,
    path: String,
    log: &'l Log,
}

impl Probe<'_, '_> {
    fn note(&self, hint: Hint) {
        self.log
            .borrow_mut()
            .entry(self.path.clone())
            .or_default()
            .push(hint);
    }
}

macro_rules! noted {
    ($($method:ident => $hint:ident),* $(,)?) => {
        $(fn $method<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
            self.note(Hint::$hint);
            self.deserialize_any(visitor)
        })*
    };
}

impl<'de> Deserializer<'de> for Probe<'_, '_> {
    type Error = serde_json::Error;

    fn deserialize_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        match self.value {
            Value::Null => visitor.visit_unit(),
            Value::Bool(value) => visitor.visit_bool(*value),
            Value::Number(number) => {
                if let Some(value) = number.as_u64() {
                    visitor.visit_u64(value)
                } else if let Some(value) = number.as_i64() {
                    visitor.visit_i64(value)
                } else {
                    visitor.visit_f64(number.as_f64().unwrap_or(f64::NAN))
                }
            }
            Value::String(value) => visitor.visit_str(value),
            Value::Array(items) => visitor.visit_seq(Items {
                items: items.iter().enumerate(),
                path: &self.path,
                log: self.log,
            }),
            Value::Object(map) => visitor.visit_map(Fields {
                entries: map.iter(),
                pending: None,
                path: &self.path,
                log: self.log,
            }),
        }
    }

    noted! {
        deserialize_bool => Bool,
        deserialize_i8 => Int, deserialize_i16 => Int, deserialize_i32 => Int,
        deserialize_i64 => Int, deserialize_i128 => Int,
        deserialize_u8 => Int, deserialize_u16 => Int, deserialize_u32 => Int,
        deserialize_u64 => Int, deserialize_u128 => Int,
        deserialize_f32 => Float, deserialize_f64 => Float,
        deserialize_char => Char, deserialize_str => Str, deserialize_string => Str,
        deserialize_unit => Unit,
        deserialize_seq => Seq,
        deserialize_map => Map,
    }

    fn deserialize_bytes<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        self.deserialize_any(visitor)
    }

    fn deserialize_byte_buf<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        self.deserialize_any(visitor)
    }

    fn deserialize_option<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        self.note(Hint::Option);
        match self.value {
            Value::Null => visitor.visit_none(),
            _ => visitor.visit_some(self),
        }
    }

    fn deserialize_unit_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        visitor: V,
    ) -> Result<V::Value, Self::Error> {
        self.deserialize_unit(visitor)
    }

    fn deserialize_newtype_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        visitor: V,
    ) -> Result<V::Value, Self::Error> {
        self.note(Hint::Newtype);
        visitor.visit_newtype_struct(self)
    }

    fn deserialize_tuple<V: Visitor<'de>>(
        self,
        len: usize,
        visitor: V,
    ) -> Result<V::Value, Self::Error> {
        self.note(Hint::Tuple(len));
        self.deserialize_any(visitor)
    }

    fn deserialize_tuple_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        len: usize,
        visitor: V,
    ) -> Result<V::Value, Self::Error> {
        self.deserialize_tuple(len, visitor)
    }

    fn deserialize_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Self::Error> {
        self.note(Hint::Struct(fields));
        self.deserialize_any(visitor)
    }

    fn deserialize_enum<V: Visitor<'de>>(
        self,
        _name: &'static str,
        variants: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Self::Error> {
        self.note(Hint::Enum(variants));
        match self.value {
            Value::String(name) => visitor.visit_enum(
                IntoDeserializer::<Self::Error>::into_deserializer(name.as_str()),
            ),
            Value::Object(map) if map.len() == 1 => {
                let (name, value) = map.iter().next().expect("one entry");
                let path = join(&self.path, name);
                visitor.visit_enum(Variant {
                    name,
                    content: Probe {
                        value,
                        path,
                        log: self.log,
                    },
                })
            }
            _ => Err(de::Error::custom("expected an externally tagged enum")),
        }
    }

    fn deserialize_identifier<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        self.deserialize_any(visitor)
    }

    fn deserialize_ignored_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        visitor.visit_unit()
    }
}

struct Items<'v, 'l, 'p> {
    items: std::iter::Enumerate<std::slice::Iter<'v, Value>>,
    path: &'p str,
    log: &'l Log,
}

impl<'de> SeqAccess<'de> for Items<'_, '_, '_> {
    type Error = serde_json::Error;

    fn next_element_seed<T: DeserializeSeed<'de>>(
        &mut self,
        seed: T,
    ) -> Result<Option<T::Value>, Self::Error> {
        let Some((index, value)) = self.items.next() else {
            return Ok(None);
        };
        seed.deserialize(Probe {
            value,
            path: format!("{}/{index}", self.path),
            log: self.log,
        })
        .map(Some)
    }
}

struct Fields<'v, 'l, 'p> {
    entries: serde_json::map::Iter<'v>,
    pending: Option<(&'v String, &'v Value)>,
    path: &'p str,
    log: &'l Log,
}

impl<'de> MapAccess<'de> for Fields<'_, '_, '_> {
    type Error = serde_json::Error;

    fn next_key_seed<K: DeserializeSeed<'de>>(
        &mut self,
        seed: K,
    ) -> Result<Option<K::Value>, Self::Error> {
        let Some((key, value)) = self.entries.next() else {
            return Ok(None);
        };
        self.pending = Some((key, value));
        seed.deserialize(IntoDeserializer::<Self::Error>::into_deserializer(
            key.as_str(),
        ))
        .map(Some)
    }

    fn next_value_seed<V: DeserializeSeed<'de>>(
        &mut self,
        seed: V,
    ) -> Result<V::Value, Self::Error> {
        let (key, value) = self
            .pending
            .take()
            .ok_or_else(|| de::Error::custom("map value requested before its key"))?;
        seed.deserialize(Probe {
            value,
            path: join(self.path, key),
            log: self.log,
        })
    }
}

struct Variant<'v, 'l> {
    name: &'v str,
    content: Probe<'v, 'l>,
}

impl<'de, 'v, 'l> EnumAccess<'de> for Variant<'v, 'l> {
    type Error = serde_json::Error;
    type Variant = Probe<'v, 'l>;

    fn variant_seed<V: DeserializeSeed<'de>>(
        self,
        seed: V,
    ) -> Result<(V::Value, Self::Variant), Self::Error> {
        let variant = seed.deserialize(IntoDeserializer::<Self::Error>::into_deserializer(
            self.name,
        ))?;
        Ok((variant, self.content))
    }
}

impl<'de> VariantAccess<'de> for Probe<'_, '_> {
    type Error = serde_json::Error;

    fn unit_variant(self) -> Result<(), Self::Error> {
        Ok(())
    }

    fn newtype_variant_seed<T: DeserializeSeed<'de>>(
        self,
        seed: T,
    ) -> Result<T::Value, Self::Error> {
        seed.deserialize(self)
    }

    fn tuple_variant<V: Visitor<'de>>(
        self,
        len: usize,
        visitor: V,
    ) -> Result<V::Value, Self::Error> {
        self.deserialize_tuple(len, visitor)
    }

    fn struct_variant<V: Visitor<'de>>(
        self,
        fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Self::Error> {
        self.deserialize_struct("", fields, visitor)
    }
}

/// Deserialize `doc` as `T` through the recorder: the result, and what serde
/// asked for at every path it visited.
fn probe<T: DeserializeOwned>(doc: &Value) -> (bool, HashMap<String, Vec<Hint>>) {
    let log = Log::default();
    let ok = T::deserialize(Probe {
        value: doc,
        path: String::new(),
        log: &log,
    })
    .is_ok();
    (ok, log.into_inner())
}

fn parses<T: DeserializeOwned>(doc: &Value) -> bool {
    T::deserialize(doc).is_ok()
}

/// One change of one slot of the document.
#[derive(Clone, Debug)]
pub(crate) enum Edit {
    /// Set the slot at the JSON pointer, inserting a field that was absent.
    Set(String, Value),
    /// Drop the last element of the array at the pointer.
    Pop(String),
}

impl Edit {
    pub(crate) fn apply(&self, base: &Value) -> Value {
        let mut doc = base.clone();
        match self {
            Edit::Set(pointer, value) => set(&mut doc, pointer, value.clone()),
            Edit::Pop(pointer) => {
                if let Some(Value::Array(items)) = doc.pointer_mut(pointer) {
                    items.pop();
                }
            }
        }
        doc
    }
}

fn set(doc: &mut Value, pointer: &str, value: Value) {
    let Some((parent, key)) = pointer.rsplit_once('/') else {
        *doc = value;
        return;
    };
    match doc.pointer_mut(parent) {
        Some(Value::Object(map)) => {
            map.insert(unescape(key), value);
        }
        Some(Value::Array(items)) => {
            if let Some(slot) = key.parse::<usize>().ok().and_then(|i| items.get_mut(i)) {
                *slot = value;
            }
        }
        _ => panic!("no container at {parent} for {pointer}"),
    }
}

/// The mutations of one leaf.
#[derive(Debug)]
pub(crate) struct Leaf {
    /// JSON pointer of the leaf plus what kind of change it is.
    pub(crate) name: String,
    pub(crate) candidates: Vec<Edit>,
    /// Run every candidate (enum variants) instead of the first accepted one.
    pub(crate) exhaustive: bool,
}

struct Walker<'a, T> {
    base: &'a Value,
    hints: HashMap<String, Vec<Hint>>,
    strings: &'a [&'a str],
    leaves: Vec<Leaf>,
    _type: std::marker::PhantomData<T>,
}

/// Every leaf of `base` (a serialized `T`) with its candidate edits.
/// `strings` are tried first for every string leaf, before a suffixed copy
/// of the current value, so a test can offer column ids or font families
/// that are valid and different.
pub(crate) fn leaves<T: DeserializeOwned>(base: &Value, strings: &[&str]) -> Vec<Leaf> {
    let (ok, hints) = probe::<T>(base);
    assert!(ok, "the baseline document must deserialize");
    let mut walker = Walker::<T> {
        base,
        hints,
        strings,
        leaves: Vec::new(),
        _type: std::marker::PhantomData,
    };
    walker.walk(base, String::new());
    walker.leaves
}

impl<T: DeserializeOwned> Walker<'_, T> {
    fn hints(&self, path: &str) -> &[Hint] {
        self.hints.get(path).map_or(&[], Vec::as_slice)
    }

    fn push(&mut self, name: String, candidates: Vec<Edit>, exhaustive: bool) {
        let candidates: Vec<Edit> = candidates
            .into_iter()
            .filter(|edit| parses::<T>(&edit.apply(self.base)))
            .collect();
        self.leaves.push(Leaf {
            name,
            candidates,
            exhaustive,
        });
    }

    fn walk(&mut self, node: &Value, path: String) {
        let hints = self.hints(&path).to_vec();
        if let Some(variants) = hints.iter().find_map(|hint| match hint {
            Hint::Enum(variants) => Some(*variants),
            _ => None,
        }) {
            self.walk_enum(node, path, variants);
            return;
        }
        if hints.contains(&Hint::Option) {
            if node.is_null() {
                let (values, exhaustive) = synthesize::<T>(self.base, &path, self.strings);
                let edits = values
                    .into_iter()
                    .map(|value| Edit::Set(path.clone(), value))
                    .collect();
                self.push(format!("{path} (None -> Some)"), edits, exhaustive);
                return;
            }
            self.push(
                format!("{path} (Some -> None)"),
                vec![Edit::Set(path.clone(), Value::Null)],
                false,
            );
        }
        match node {
            Value::Null => {}
            Value::Bool(value) => self.push(
                path.clone(),
                vec![Edit::Set(path, Value::Bool(!value))],
                false,
            ),
            Value::Number(number) => {
                let edits = number_candidates(number)
                    .into_iter()
                    .map(|value| Edit::Set(path.clone(), value))
                    .collect();
                self.push(path, edits, false);
            }
            Value::String(value) => {
                if hints.contains(&Hint::Char) {
                    let other = if value == "Q" { "R" } else { "Q" };
                    self.push(
                        path.clone(),
                        vec![Edit::Set(path, Value::from(other))],
                        false,
                    );
                } else {
                    self.walk_string(value, path);
                }
            }
            Value::Array(items) => {
                for (index, item) in items.iter().enumerate() {
                    self.walk(item, format!("{path}/{index}"));
                }
                if items.is_empty() {
                    let mut doc = self.base.clone();
                    set(&mut doc, &path, Value::Array(vec![sentinel()]));
                    let element = format!("{path}/0");
                    let (values, exhaustive) = synthesize::<T>(&doc, &element, self.strings);
                    let edits = values
                        .into_iter()
                        .map(|value| Edit::Set(path.clone(), Value::Array(vec![value])))
                        .collect();
                    self.push(format!("{path} (push)"), edits, exhaustive);
                } else {
                    self.push(
                        format!("{path} (pop)"),
                        vec![Edit::Pop(path.clone())],
                        false,
                    );
                }
            }
            Value::Object(map) => {
                let fields = hints.iter().find_map(|hint| match hint {
                    Hint::Struct(fields) => Some(*fields),
                    _ => None,
                });
                for field in fields.unwrap_or(&[]) {
                    if map.contains_key(*field) {
                        continue;
                    }
                    let slot = join(&path, field);
                    let (values, exhaustive) = synthesize::<T>(self.base, &slot, self.strings);
                    let edits = values
                        .into_iter()
                        .map(|value| Edit::Set(slot.clone(), value))
                        .collect();
                    self.push(format!("{slot} (absent -> set)"), edits, exhaustive);
                }
                for (key, value) in map {
                    self.walk(value, join(&path, key));
                }
            }
        }
    }

    fn walk_enum(&mut self, node: &Value, path: String, variants: &'static [&'static str]) {
        let current = match node {
            Value::String(name) => Some(name.as_str()),
            Value::Object(map) if map.len() == 1 => map.keys().next().map(String::as_str),
            _ => None,
        };
        let mut edits = Vec::new();
        for variant in variants.iter().filter(|v| Some(**v) != current) {
            if let Some(value) = enum_variant::<T>(self.base, &path, variant, node, self.strings) {
                edits.push(Edit::Set(path.clone(), value));
            }
        }
        self.push(format!("{path} (variant)"), edits, true);
        if let Value::Object(map) = node
            && let Some((name, content)) = map.iter().next()
        {
            self.walk(content, join(&path, name));
        }
    }

    fn walk_string(&mut self, value: &str, path: String) {
        // An internally tagged enum's tag is a plain string to the recorder;
        // serde names the other variants when the tag is unknown.
        let mut doc = self.base.clone();
        set(&mut doc, &path, sentinel());
        if let Err(error) = T::deserialize(&doc) {
            let message = error.to_string();
            if message.contains(&format!("unknown variant `{SENTINEL}`")) {
                let edits = expected_variants(&message)
                    .into_iter()
                    .filter(|variant| variant != value)
                    .map(|variant| Edit::Set(path.clone(), Value::String(variant)))
                    .collect();
                self.push(format!("{path} (tag)"), edits, true);
                return;
            }
        }
        let mut edits: Vec<Edit> = self
            .strings
            .iter()
            .filter(|s| **s != value)
            .map(|s| Edit::Set(path.clone(), Value::String((*s).to_string())))
            .collect();
        edits.push(Edit::Set(path.clone(), Value::String(format!("{value}_m"))));
        // Each replacement is a different input (a font family, a column).
        self.push(path, edits, true);
    }
}

/// The variant list in serde's "unknown variant `x`, expected one of `a`,
/// `b`" (or "expected `a` or `b`") message.
fn expected_variants(message: &str) -> Vec<String> {
    let Some((_, tail)) = message.split_once("expected ") else {
        return Vec::new();
    };
    tail.split('`')
        .skip(1)
        .step_by(2)
        .map(str::to_string)
        .collect()
}

fn number_candidates(number: &serde_json::Number) -> Vec<Value> {
    if let Some(n) = number.as_u64() {
        let mut out = vec![Value::from(n + 1), Value::from(n + 2)];
        if n > 0 {
            out.push(Value::from(n - 1));
        }
        return out;
    }
    if let Some(n) = number.as_i64() {
        return vec![Value::from(n + 1), Value::from(n - 1)];
    }
    let x = number.as_f64().unwrap_or(0.0);
    let floats: &[f64] = if x == 0.0 {
        &[0.5, 1.0, -0.5]
    } else {
        &[x * 1.5, x * 0.5, -x, x + 1.0]
    };
    floats.iter().map(|f| Value::from(*f)).collect()
}

/// The value of `variant` for the enum at `path`, whose current value is
/// `node`: the current content under the new tag when that parses, otherwise
/// the variant built by [`variant_value`] from the current content.
fn enum_variant<T: DeserializeOwned>(
    base: &Value,
    path: &str,
    variant: &str,
    node: &Value,
    strings: &[&str],
) -> Option<Value> {
    let content = match node {
        Value::Object(map) if map.len() == 1 => map.values().next(),
        _ => None,
    };
    if let Some(content) = content {
        let value = wrap(variant, content.clone());
        let mut doc = base.clone();
        set(&mut doc, path, value.clone());
        if parses::<T>(&doc) {
            return Some(value);
        }
    }
    let value = variant_value::<T>(base, path, variant, content, strings);
    let mut doc = base.clone();
    set(&mut doc, path, value.clone());
    parses::<T>(&doc).then_some(value)
}

fn wrap(variant: &str, content: Value) -> Value {
    let mut map = serde_json::Map::new();
    map.insert(variant.into(), content);
    Value::Object(map)
}

/// What serde asks for at `path` when the slot holds the sentinel.
fn slot_hints<T: DeserializeOwned>(doc: &Value, path: &str) -> Vec<Hint> {
    let mut probe_doc = doc.clone();
    set(&mut probe_doc, path, sentinel());
    let (_, mut hints) = probe::<T>(&probe_doc);
    hints.remove(path).unwrap_or_default()
}

/// A value of `variant` for the enum at `path`, by type alone: the unit
/// variant, or the variant's content with every field given a value (the
/// fields of `content` it shares kept).
fn variant_value<T: DeserializeOwned>(
    doc: &Value,
    path: &str,
    variant: &str,
    content: Option<&Value>,
    strings: &[&str],
) -> Value {
    let mut doc = doc.clone();
    set(&mut doc, path, wrap(variant, Value::Null));
    let slot = join(path, variant);
    let hints = slot_hints::<T>(&doc, &slot);
    if hints.is_empty() {
        // The unit variant reads no content.
        return Value::String(variant.into());
    }
    if let Some(fields) = struct_fields(&hints) {
        let start = match content {
            Some(Value::Object(map)) => Value::Object(map.clone()),
            _ => Value::Object(Default::default()),
        };
        set(&mut doc, &slot, start);
        fill_fields::<T>(&mut doc, &slot, fields, strings);
        return wrap(variant, doc.pointer(&slot).cloned().unwrap_or(Value::Null));
    }
    let value = shape::<T>(&doc, &slot, strings)
        .0
        .into_iter()
        .next()
        .unwrap_or(Value::Null);
    wrap(variant, value)
}

fn struct_fields(hints: &[Hint]) -> Option<&'static [&'static str]> {
    hints.iter().find_map(|hint| match hint {
        Hint::Struct(fields) => Some(*fields),
        _ => None,
    })
}

/// Give every field of the struct at `path` that the object lacks a value
/// of its type.
fn fill_fields<T: DeserializeOwned>(
    doc: &mut Value,
    path: &str,
    fields: &[&str],
    strings: &[&str],
) {
    for field in fields {
        let slot = join(path, field);
        if doc.pointer(&slot).is_some() {
            continue;
        }
        // An optional field is valid as `None`; leave it to its own leaf.
        if slot_hints::<T>(doc, &slot).first() == Some(&Hint::Option) {
            set(doc, &slot, Value::Null);
            continue;
        }
        if let Some(value) = shape::<T>(doc, &slot, strings).0.into_iter().next() {
            set(doc, &slot, value);
        }
    }
}

/// Values of the type serde asks for at `path` of `doc`, by type alone (the
/// caller checks the whole document); `true` when they are the variants of
/// an enum.
fn shape<T: DeserializeOwned>(doc: &Value, path: &str, strings: &[&str]) -> (Vec<Value>, bool) {
    let hints = slot_hints::<T>(doc, path);
    let hint = hints
        .iter()
        .find(|hint| !matches!(hint, Hint::Option | Hint::Newtype))
        .cloned();
    let values = match hint {
        Some(Hint::Bool) => vec![Value::Bool(true), Value::Bool(false)],
        // Numbers grow in field order across one synthesized value, so a
        // `min` declared before its `max` comes out as a valid range.
        Some(Hint::Int) => {
            let n = next_number();
            vec![Value::from(n), Value::from(n + 1)]
        }
        Some(Hint::Float) => {
            // Increasing and inside (0, 1), where colours and fractions live.
            let n = next_number() as f64;
            vec![
                Value::from(n / (n + 1.0)),
                Value::from(12.0),
                Value::from(n),
            ]
        }
        Some(Hint::Char) => vec![Value::from("Q")],
        Some(Hint::Str) => strings
            .iter()
            .map(|s| Value::from(*s))
            .chain([Value::from("m")])
            .collect(),
        Some(Hint::Unit) => vec![Value::Null],
        Some(Hint::Map) => vec![Value::Object(Default::default())],
        Some(Hint::Seq) => {
            // One element first, so an absent list becomes a different one.
            let mut doc = doc.clone();
            set(&mut doc, path, Value::Array(vec![sentinel()]));
            let element = shape::<T>(&doc, &format!("{path}/0"), strings).0;
            element
                .into_iter()
                .take(1)
                .map(|value| Value::Array(vec![value]))
                .chain([Value::Array(Vec::new())])
                .collect()
        }
        Some(Hint::Tuple(len)) => {
            let mut doc = doc.clone();
            set(&mut doc, path, Value::Array(vec![sentinel(); len]));
            let mut items = Vec::new();
            for index in 0..len {
                let element = format!("{path}/{index}");
                let value = shape::<T>(&doc, &element, strings).0.into_iter().next();
                let value = value.unwrap_or(Value::Null);
                set(&mut doc, &element, value.clone());
                items.push(value);
            }
            vec![Value::Array(items)]
        }
        Some(Hint::Struct(fields)) => {
            let mut doc = doc.clone();
            set(&mut doc, path, Value::Object(Default::default()));
            fill_fields::<T>(&mut doc, path, fields, strings);
            doc.pointer(path).cloned().into_iter().collect()
        }
        Some(Hint::Enum(variants)) => {
            let values = variants
                .iter()
                .map(|variant| variant_value::<T>(doc, path, variant, None, strings))
                .collect();
            return (values, true);
        }
        Some(Hint::Option | Hint::Newtype) | None => {
            if let Some(values) = internally_tagged::<T>(doc, path, strings) {
                return (values, true);
            }
            vec![
                Value::Bool(true),
                Value::from(1u64),
                Value::from(0.5),
                Value::from("m"),
                Value::Array(Vec::new()),
                Value::Object(Default::default()),
            ]
        }
    };
    (values, false)
}

/// The variants of an internally tagged enum at `path`, whose content serde
/// buffers out of the recorder's sight: an empty object names the tag field
/// ("missing field `mode`"), an unknown tag names the variants, and each
/// variant's own missing fields are filled from the fallback values.
fn internally_tagged<T: DeserializeOwned>(
    doc: &Value,
    path: &str,
    strings: &[&str],
) -> Option<Vec<Value>> {
    let error = |doc: &Value| T::deserialize(doc).err().map(|error| error.to_string());
    let missing = |message: &str| {
        message
            .split_once("missing field `")
            .and_then(|(_, tail)| tail.split_once('`'))
            .map(|(field, _)| field.to_string())
    };
    let mut doc = doc.clone();
    set(&mut doc, path, Value::Object(Default::default()));
    let tag = missing(&error(&doc)?)?;
    let mut object = serde_json::Map::new();
    object.insert(tag.clone(), sentinel());
    set(&mut doc, path, Value::Object(object));
    let message = error(&doc)?;
    if !message.contains(&format!("unknown variant `{SENTINEL}`")) {
        return None;
    }
    let fallback = [
        Value::from(strings.first().copied().unwrap_or("m")),
        Value::from(0.5),
        Value::Bool(true),
        Value::from(1u64),
        Value::Array(Vec::new()),
        Value::Object(Default::default()),
        Value::Null,
    ];
    let mut values = Vec::new();
    for variant in expected_variants(&message) {
        let mut object = serde_json::Map::new();
        object.insert(tag.clone(), Value::String(variant));
        set(&mut doc, path, Value::Object(object));
        for _ in 0..32 {
            let Some(message) = error(&doc) else { break };
            let Some(field) = missing(&message) else {
                break;
            };
            let slot = join(path, &field);
            if doc.pointer(&slot).is_some() {
                break;
            }
            let value = fallback.iter().find(|value| {
                let mut trial = doc.clone();
                set(&mut trial, &slot, (*value).clone());
                error(&trial).is_none_or(|next| missing(&next).is_some_and(|other| other != field))
            });
            set(&mut doc, &slot, value.cloned().unwrap_or(Value::Null));
        }
        values.extend(doc.pointer(path).cloned());
    }
    Some(values)
}

/// Candidate values for the slot at `path` of `doc` (an absent field, a
/// `None`, a new array element); `true` when they are enum variants.
fn synthesize<T: DeserializeOwned>(
    doc: &Value,
    path: &str,
    strings: &[&str],
) -> (Vec<Value>, bool) {
    NUMBERS.with(|n| n.set(1));
    shape::<T>(doc, path, strings)
}

thread_local! {
    static NUMBERS: std::cell::Cell<u64> = const { std::cell::Cell::new(1) };
}

fn next_number() -> u64 {
    NUMBERS.with(|n| {
        let value = n.get();
        n.set(value + 1);
        value
    })
}

/// What a check made of one mutated input.
#[derive(Debug)]
pub(crate) enum Outcome {
    /// The renderer's validation rejected the input; try the next candidate.
    Rejected,
    /// The property holds for this mutation.
    Held,
    /// The output changed and the key did not; the evidence.
    Gap(String),
}

/// Counts for the report and the failure list for the assertion.
#[derive(Debug, Default)]
pub(crate) struct Tally {
    pub(crate) leaves: usize,
    pub(crate) mutations: usize,
    /// Leaves whose every candidate was rejected or unrepresentable.
    pub(crate) skipped: Vec<String>,
    /// Candidates that deserialize to the baseline value itself.
    pub(crate) no_ops: usize,
    /// One entry per failing leaf.
    pub(crate) gaps: Vec<String>,
}

impl Tally {
    pub(crate) fn absorb(&mut self, other: Tally) {
        self.leaves += other.leaves;
        self.mutations += other.mutations;
        self.skipped.extend(other.skipped);
        self.no_ops += other.no_ops;
        self.gaps.extend(other.gaps);
    }

    pub(crate) fn summary(&self, name: &str) -> String {
        format!(
            "{name}: {} leaves, {} mutations run, {} leaves skipped as invalid, \
             {} no-op candidates, {} failing leaves",
            self.leaves,
            self.mutations,
            self.skipped.len(),
            self.no_ops,
            self.gaps.len()
        )
    }
}

/// Run `check` on one mutation per leaf of `base` (every variant for an
/// enum leaf). `label` prefixes every reported path.
pub(crate) fn run<T>(
    label: &str,
    base: &T,
    strings: &[&str],
    mut check: impl FnMut(&str, T) -> Outcome,
) -> Tally
where
    T: Serialize + DeserializeOwned + PartialEq,
{
    let doc = serde_json::to_value(base).expect("the baseline serializes");
    let mut tally = Tally::default();
    for leaf in leaves::<T>(&doc, strings) {
        tally.leaves += 1;
        let name = format!("{label}{}", leaf.name);
        let mut ran = 0usize;
        // One entry per leaf: the first failing candidate's evidence.
        let mut gap: Option<String> = None;
        let mut failing = 0usize;
        for edit in &leaf.candidates {
            let value: T =
                serde_json::from_value(edit.apply(&doc)).expect("candidates are filtered to parse");
            if value == *base {
                tally.no_ops += 1;
                continue;
            }
            let shown = match edit {
                Edit::Set(_, value) => compact(value),
                Edit::Pop(_) => "pop".into(),
            };
            match check(&format!("{name} = {shown}"), value) {
                Outcome::Rejected => continue,
                Outcome::Held => {}
                Outcome::Gap(evidence) => {
                    failing += 1;
                    gap.get_or_insert(format!("{name} = {shown}: {evidence}"));
                }
            }
            tally.mutations += 1;
            ran += 1;
            if !leaf.exhaustive {
                break;
            }
        }
        if let Some(gap) = gap {
            tally.gaps.push(if ran > 1 {
                format!("{gap} [{failing} of {ran} values of this leaf fail]")
            } else {
                gap
            });
        }
        if ran == 0 {
            tally.skipped.push(name);
        }
    }
    tally
}

fn compact(value: &Value) -> String {
    let text = value.to_string();
    if text.chars().count() > 60 {
        format!("{}...", text.chars().take(60).collect::<String>())
    } else {
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
    enum Kind {
        Unit,
        Data { value: f32, flag: bool },
        Wrapped(f64),
    }

    #[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
    struct Inner {
        count: u32,
        name: String,
    }

    #[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
    struct Doc {
        flag: bool,
        kind: Kind,
        list: Vec<Inner>,
        inner: Option<Inner>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        hidden: Option<Kind>,
    }

    // Every leaf is found, including the field that serializes to nothing,
    // and each candidate parses.
    #[test]
    fn walker_finds_every_leaf_including_absent_fields() {
        let doc = Doc {
            flag: true,
            kind: Kind::Data {
                value: 1.0,
                flag: false,
            },
            list: vec![Inner {
                count: 2,
                name: "a".into(),
            }],
            inner: None,
            hidden: None,
        };
        let value = serde_json::to_value(&doc).unwrap();
        let found = leaves::<Doc>(&value, &["b"]);
        let names: Vec<&str> = found.iter().map(|leaf| leaf.name.as_str()).collect();
        for expected in [
            "/flag",
            "/kind (variant)",
            "/kind/Data/value",
            "/kind/Data/flag",
            "/list/0/count",
            "/list/0/name",
            "/list (pop)",
            "/inner (None -> Some)",
            "/hidden (absent -> set)",
        ] {
            assert!(
                names.contains(&expected),
                "{expected} missing from {names:?}"
            );
        }
        let variants = found
            .iter()
            .find(|leaf| leaf.name == "/kind (variant)")
            .unwrap();
        assert_eq!(variants.candidates.len(), 2, "Unit and Wrapped");
        let hidden = found
            .iter()
            .find(|leaf| leaf.name == "/hidden (absent -> set)")
            .unwrap();
        assert_eq!(
            hidden.candidates.len(),
            3,
            "every variant of the absent enum"
        );
        // The runner moves to the next candidate after a rejection, records
        // a gap with its leaf, and skips nothing else here.
        let mut rejected = false;
        let tally = run("doc: ", &doc, &["b"], |label, mutated| {
            assert_ne!(mutated, doc);
            if label.starts_with("doc: /list/0/count ") && !rejected {
                rejected = true;
                return Outcome::Rejected;
            }
            if label.starts_with("doc: /flag ") {
                return Outcome::Gap("evidence".into());
            }
            Outcome::Held
        });
        assert!(rejected);
        assert!(tally.skipped.is_empty(), "{:?}", tally.skipped);
        assert_eq!(tally.gaps, ["doc: /flag = false: evidence"]);
        assert_eq!(tally.leaves, found.len());
        let mut total = Tally::default();
        total.absorb(tally);
        assert!(
            total.summary("doc").ends_with("1 failing leaves"),
            "{}",
            total.summary("doc")
        );
    }
}
