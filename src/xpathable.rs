/// Very simplified XPath implementation for serde_json and serde_yaml.
use anyhow::{Result, anyhow};

enum Index<'a> {
    Number(usize),
    String(&'a str),
}

pub trait XPathable<T> where Self: std::fmt::Debug {
    fn xpath(&self, path: &str) -> Result<&Self> {
        let mut obj = self;
        let mut paths = path.split_terminator('/');
        paths.next();
        for new_index in paths {
            // verify that we have an object
            if let Ok(index) = new_index.parse::<usize>() {
                obj = obj.get_next(Index::Number(index))
                    .ok_or(anyhow!("Unable to find index: {}", new_index))?;
            } else {
                obj = obj.get_next(Index::String(new_index))
                    .ok_or(anyhow!("Unable to find key: {}", new_index))?;
            }
        }
        Ok(obj)
    }

    fn get_next<'a>(&'a self, key: Index) -> Option<&'a Self>;
}

mod json {
    use serde_json::value::{Value, Index as JIndex};
    use super::{XPathable, Index};

    impl XPathable<Value> for Value {
        fn get_next<'a>(&'a self, key: Index) ->
                Option<&'a Value> {
            match key {
                Index::Number(key) => self.get::<'a>(key),
                Index::String(key) => self.get(key),
            }
        }
    }
}
