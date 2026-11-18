//! JS-object helpers. The planner's spec is the JS object literal the archetype
//! modules write, kept as a `serde_json::Value` so a port reads like its
//! source: a missing key is `undefined`, `Value::Null` is `null`.

pub use serde_json::{Map, Value as J, json};

/// Read access with JS semantics.
pub trait JExt {
    /// The value at `key` unless it is missing (`undefined`). `null` is returned.
    fn at(&self, key: &str) -> Option<&J>;
    /// `obj[key] ?? fallback`-style read: missing or null is None.
    fn get_nn(&self, key: &str) -> Option<&J> {
        self.at(key).filter(|v| !v.is_null())
    }
    /// A finite number at `key` (missing, null, NaN or non-number: None).
    fn f(&self, key: &str) -> Option<f64> {
        self.get_nn(key).and_then(|v| v.as_f64()).filter(|v| v.is_finite())
    }
    /// `Number(obj[key])`: like JS for numbers, numeric strings and booleans.
    fn num(&self, key: &str) -> f64 {
        to_number(self.at(key))
    }
    fn s(&self, key: &str) -> Option<&str> {
        self.get_nn(key).and_then(|v| v.as_str())
    }
    fn b(&self, key: &str) -> Option<bool> {
        self.get_nn(key).and_then(|v| v.as_bool())
    }
    /// A JS-truthy test of `obj[key]`.
    fn truthy(&self, key: &str) -> bool {
        truthy(self.at(key))
    }
    fn obj(&self, key: &str) -> Option<&Map<String, J>> {
        self.get_nn(key).and_then(|v| v.as_object())
    }
    fn arr(&self, key: &str) -> Option<&Vec<J>> {
        self.get_nn(key).and_then(|v| v.as_array())
    }
    /// `key in obj && obj[key] !== undefined`.
    fn has(&self, key: &str) -> bool {
        self.at(key).is_some()
    }
}

impl JExt for J {
    fn at(&self, key: &str) -> Option<&J> {
        self.as_object().and_then(|m| m.get(key))
    }
}

impl JExt for Map<String, J> {
    fn at(&self, key: &str) -> Option<&J> {
        self.get(key)
    }
}

impl JExt for Option<&J> {
    fn at(&self, key: &str) -> Option<&J> {
        self.and_then(|v| v.at(key))
    }
}

/// JS `Number(v)` for a JSON value (`undefined` -> NaN).
pub fn to_number(v: Option<&J>) -> f64 {
    match v {
        None => f64::NAN,
        Some(J::Null) => 0.0,
        Some(J::Bool(b)) => *b as u8 as f64,
        Some(J::Number(n)) => n.as_f64().unwrap_or(f64::NAN),
        Some(J::String(s)) => {
            let t = s.trim();
            if t.is_empty() { 0.0 } else { t.parse::<f64>().unwrap_or(f64::NAN) }
        }
        Some(J::Array(a)) => match a.len() {
            0 => 0.0,
            1 => to_number(Some(&a[0])),
            _ => f64::NAN,
        },
        Some(J::Object(_)) => f64::NAN,
    }
}

/// JS truthiness.
pub fn truthy(v: Option<&J>) -> bool {
    match v {
        None | Some(J::Null) => false,
        Some(J::Bool(b)) => *b,
        Some(J::Number(n)) => n.as_f64().map(|x| x != 0.0 && !x.is_nan()).unwrap_or(false),
        Some(J::String(s)) => !s.is_empty(),
        Some(_) => true,
    }
}

/// `Number(x) || fallback`.
pub fn num_or(v: Option<&J>, fallback: f64) -> f64 {
    let n = to_number(v);
    if n == 0.0 || n.is_nan() { fallback } else { n }
}

/// A JSON number (NaN and infinities become null, as JSON.stringify writes them).
pub fn jn(v: f64) -> J {
    serde_json::Number::from_f64(v).map(J::Number).unwrap_or(J::Null)
}

/// `merge(defaults, override)` in kit/elevation.js: `false`/`null` -> None,
/// `undefined`/`true` -> a copy of the defaults, an object -> spread over them.
pub fn merge(defaults: &J, over: Option<&J>) -> Option<J> {
    match over {
        Some(J::Bool(false)) | Some(J::Null) => None,
        None | Some(J::Bool(true)) => Some(defaults.clone()),
        Some(J::Object(o)) => {
            let mut out = defaults.as_object().cloned().unwrap_or_default();
            for (k, v) in o {
                out.insert(k.clone(), v.clone());
            }
            Some(J::Object(out))
        }
        // A number or string spreads to nothing in JS: `{ ...defaults, ...5 }`.
        Some(_) => Some(defaults.clone()),
    }
}

/// `{ ...a, ...b }` for two JSON objects (non-objects spread to nothing).
pub fn spread(a: Option<&J>, b: Option<&J>) -> J {
    let mut out = a.and_then(|v| v.as_object()).cloned().unwrap_or_default();
    if let Some(o) = b.and_then(|v| v.as_object()) {
        for (k, v) in o {
            out.insert(k.clone(), v.clone());
        }
    }
    J::Object(out)
}

/// Set `obj[key] = value` on a JSON object (creating the object if needed).
pub fn set(obj: &mut J, key: &str, value: J) {
    if !obj.is_object() {
        *obj = J::Object(Map::new());
    }
    obj.as_object_mut().unwrap().insert(key.to_string(), value);
}

/// `obj[key]` as a mutable object, created when missing.
pub fn obj_mut<'a>(obj: &'a mut J, key: &str) -> &'a mut J {
    if !obj.is_object() {
        *obj = J::Object(Map::new());
    }
    let m = obj.as_object_mut().unwrap();
    let e = m.entry(key.to_string()).or_insert_with(|| J::Object(Map::new()));
    if !e.is_object() {
        *e = J::Object(Map::new());
    }
    e
}

/// JS `Math.round` (half up toward +infinity).
pub fn js_round(v: f64) -> f64 {
    (v + 0.5).floor()
}

/// `a.toFixed(digits)` for the small magnitudes keys are built from.
pub fn to_fixed(v: f64, digits: usize) -> String {
    format!("{:.*}", digits, v)
}
