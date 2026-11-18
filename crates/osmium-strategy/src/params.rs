use std::{collections::BTreeMap, fmt};

use osmium_types::{Price, Symbol};

/// One strategy parameter. Amounts with a fraction are written as text (`"12.5"`), so
/// no binary floating point is involved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    Integer(i64),
    Text(String),
    Boolean(bool),
    List(Vec<Value>),
}

/// A parameter is missing, has another type than expected, or is not known to the
/// strategy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParamError {
    pub name: String,
    pub problem: String,
}

impl ParamError {
    #[must_use]
    pub fn new(name: &str, problem: impl Into<String>) -> Self {
        Self {
            name: name.to_owned(),
            problem: problem.into(),
        }
    }
}

impl fmt::Display for ParamError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "strategy parameter `{}` {}",
            self.name, self.problem
        )
    }
}

impl std::error::Error for ParamError {}

/// The parameters of a strategy (`[strategy.params]` of a run's configuration).
///
/// Reading a parameter removes it, so what is left after a strategy is built are the
/// parameters it does not know; [`Params::finish`] reports them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Params(BTreeMap<String, Value>);

impl Params {
    #[must_use]
    pub const fn new(values: BTreeMap<String, Value>) -> Self {
        Self(values)
    }

    /// Removes and returns a parameter of any type.
    pub fn take(&mut self, name: &str) -> Option<Value> {
        self.0.remove(name)
    }

    fn required(&mut self, name: &str) -> Result<Value, ParamError> {
        self.take(name)
            .ok_or_else(|| ParamError::new(name, "is missing"))
    }

    pub fn integer(&mut self, name: &str) -> Result<i64, ParamError> {
        match self.required(name)? {
            Value::Integer(value) => Ok(value),
            _ => Err(ParamError::new(name, "must be an integer")),
        }
    }

    pub fn integer_or(&mut self, name: &str, default: i64) -> Result<i64, ParamError> {
        if self.0.contains_key(name) {
            self.integer(name)
        } else {
            Ok(default)
        }
    }

    pub fn boolean_or(&mut self, name: &str, default: bool) -> Result<bool, ParamError> {
        match self.take(name) {
            None => Ok(default),
            Some(Value::Boolean(value)) => Ok(value),
            Some(_) => Err(ParamError::new(name, "must be true or false")),
        }
    }

    pub fn text(&mut self, name: &str) -> Result<String, ParamError> {
        match self.required(name)? {
            Value::Text(value) => Ok(value),
            _ => Err(ParamError::new(name, "must be text")),
        }
    }

    /// A price written as text (`"583.5"`) or as a whole number of yuan.
    pub fn price(&mut self, name: &str) -> Result<Price, ParamError> {
        let text = match self.required(name)? {
            Value::Text(value) => value,
            Value::Integer(value) => value.to_string(),
            _ => return Err(ParamError::new(name, "must be a price")),
        };
        text.parse()
            .map_err(|_| ParamError::new(name, format!("`{text}` is not a price")))
    }

    pub fn symbol(&mut self, name: &str) -> Result<Symbol, ParamError> {
        let value = self.required(name)?;
        symbol_of(name, &value)
    }

    /// A list of symbols.
    pub fn symbols(&mut self, name: &str) -> Result<Vec<Symbol>, ParamError> {
        match self.required(name)? {
            Value::List(values) => values.iter().map(|value| symbol_of(name, value)).collect(),
            _ => Err(ParamError::new(name, "must be a list of symbols")),
        }
    }

    /// Fails when a parameter was not read: the strategy does not know it.
    pub fn finish(&self) -> Result<(), ParamError> {
        match self.0.keys().next() {
            None => Ok(()),
            Some(name) => Err(ParamError::new(name, "is not a parameter of this strategy")),
        }
    }
}

fn symbol_of(name: &str, value: &Value) -> Result<Symbol, ParamError> {
    let Value::Text(code) = value else {
        return Err(ParamError::new(name, "must be a symbol written as text"));
    };
    Symbol::new(code).map_err(|_| ParamError::new(name, format!("`{code}` is not a symbol")))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::{Params, Value};

    fn params() -> Params {
        Params::new(BTreeMap::from([
            ("qty".to_owned(), Value::Integer(2_000)),
            ("limit".to_owned(), Value::Text("583.5".to_owned())),
            (
                "symbols".to_owned(),
                Value::List(vec![Value::Text("2330".to_owned())]),
            ),
        ]))
    }

    #[test]
    fn parameters_are_typed_and_read_once() {
        let mut params = params();
        assert_eq!(params.integer("qty"), Ok(2_000));
        assert_eq!(params.integer_or("lots", 3), Ok(3));
        assert_eq!(params.price("limit").unwrap().to_string(), "583.5");
        assert_eq!(params.symbols("symbols").unwrap()[0].as_str(), "2330");
        assert_eq!(params.boolean_or("verbose", false), Ok(false));
        assert_eq!(params.finish(), Ok(()));
    }

    #[test]
    fn a_missing_mistyped_or_unknown_parameter_is_an_error() {
        let mut params = params();
        assert_eq!(
            params.integer("lots").unwrap_err().to_string(),
            "strategy parameter `lots` is missing"
        );
        assert_eq!(
            params.text("qty").unwrap_err().to_string(),
            "strategy parameter `qty` must be text"
        );
        assert_eq!(
            params.finish().unwrap_err().to_string(),
            "strategy parameter `limit` is not a parameter of this strategy"
        );
    }
}
