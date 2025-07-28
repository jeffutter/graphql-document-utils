use graphql_parser::query::{
    self, Definition, Directive, Document, Selection, Value, VariableDefinition,
};
use std::fmt::Display;

pub fn normalize(s: &str) -> Result<String, Box<dyn std::error::Error>> {
    let document = query::parse_query::<String>(s)?;
    let mut doc = Doc::new(document);
    doc.normalize();
    Ok(format!("{doc}"))
}

struct Doc<'a>(Document<'a, String>);

impl Display for Doc<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl<'a> Doc<'a> {
    fn new(document: Document<'a, String>) -> Self {
        Self(document)
    }

    fn normalize(&mut self) {
        for definition in &mut self.0.definitions {
            match definition {
                Definition::Operation(op) => match op {
                    query::OperationDefinition::SelectionSet(set) => {
                        normalize_selection_set(&mut set.items);
                    }
                    query::OperationDefinition::Query(query) => {
                        normalize_selection_set(&mut query.selection_set.items);
                        normalize_directives(&mut query.directives);
                        normalize_variable_definitions(&mut query.variable_definitions);
                    }
                    query::OperationDefinition::Mutation(mutation) => {
                        normalize_selection_set(&mut mutation.selection_set.items);
                        normalize_directives(&mut mutation.directives);
                        normalize_variable_definitions(&mut mutation.variable_definitions);
                    }
                    query::OperationDefinition::Subscription(subscription) => {
                        normalize_selection_set(&mut subscription.selection_set.items);
                        normalize_directives(&mut subscription.directives);
                        normalize_variable_definitions(&mut subscription.variable_definitions);
                    }
                },
                Definition::Fragment(frag) => {
                    normalize_selection_set(&mut frag.selection_set.items);
                    normalize_directives(&mut frag.directives);
                }
            }
        }

        self.0.definitions.sort_by_key(|d| {
            match d {
                Definition::Operation(o) => match o {
                    query::OperationDefinition::SelectionSet(_) => String::from(""),
                    query::OperationDefinition::Query(q) => {
                        let mut s = String::from("AAAA");
                        if let Some(name) = q.name.clone() {
                            s += &name;
                        }
                        s
                    }
                    query::OperationDefinition::Mutation(m) => {
                        let mut s = String::from("BBBB");
                        if let Some(name) = m.name.clone() {
                            s += &name;
                        }
                        s
                    }
                    query::OperationDefinition::Subscription(sub) => {
                        let mut s = String::from("CCCC");
                        if let Some(name) = sub.name.clone() {
                            s += &name;
                        }
                        s
                    }
                },
                Definition::Fragment(frag) => {
                    let mut s = String::from("ZZZZ");
                    s += &frag.name;
                    s
                }
            }
            .to_lowercase()
        });
    }
}

fn normalize_selection_set(selections: &mut [Selection<String>]) {
    for selection in selections.iter_mut() {
        match selection {
            Selection::Field(field) => {
                normalize_directives(&mut field.directives);
                normalize_selection_set(&mut field.selection_set.items);
                field.arguments.sort_by_key(|(k, _v)| k.to_lowercase());
            }
            Selection::FragmentSpread(frag_spread) => {
                normalize_directives(&mut frag_spread.directives);
            }
            Selection::InlineFragment(inline) => {
                normalize_selection_set(&mut inline.selection_set.items);
                normalize_directives(&mut inline.directives);
            }
        }
    }

    selections.sort_by_key(|s| {
        match s {
            Selection::Field(f) => f.name.clone(),
            Selection::FragmentSpread(fs) => {
                let mut s = String::from("ZZZZ");
                s += &fs.fragment_name;
                s
            }
            Selection::InlineFragment(f) => {
                let mut s = String::from("ZZZZZZZZ");
                if let Some(tc) = &f.type_condition {
                    match tc {
                        query::TypeCondition::On(on) => s += on,
                    }
                }
                s
            }
        }
        .to_lowercase()
    });
}

fn normalize_directives(directives: &mut [Directive<String>]) {
    for directive in directives.iter_mut() {
        for (_argument, value) in directive.arguments.iter_mut() {
            normalize_value(value);
        }
        directive.arguments.sort_by_key(|(k, _v)| k.to_lowercase());
    }

    directives.sort_by_key(|d| d.name.to_lowercase());
}

fn normalize_variable_definitions(variable_definitions: &mut [VariableDefinition<String>]) {
    for variable_definition in variable_definitions.iter_mut() {
        if let Some(default_value) = &mut variable_definition.default_value {
            normalize_value(default_value);
        }
    }

    variable_definitions.sort_by_key(|vd| vd.name.to_lowercase());
}

fn normalize_value(value: &mut Value<String>) {
    match value {
        query::Value::Variable(_) => (),
        query::Value::Int(_) => (),
        query::Value::Float(_) => (),
        query::Value::String(_) => (),
        query::Value::Boolean(_) => (),
        query::Value::Null => (),
        query::Value::Enum(_) => (),
        query::Value::List(list) => {
            for value in list.iter_mut() {
                normalize_value(value);
            }
            list.sort_by_key(|v| {
                match v {
                    Value::Variable(v) => v.clone(),
                    Value::Int(i) => i.as_i64().unwrap_or(0).to_string(),
                    Value::Float(f) => f.to_string(),
                    Value::String(s) => s.clone(),
                    Value::Boolean(_) => String::from("a"),
                    Value::Null => String::from(""),
                    Value::Enum(e) => e.to_string(),
                    Value::List(_) => String::from("ZZZZ"),
                    Value::Object(_) => String::from("ZZZZ"),
                }
                .to_lowercase()
            })
        }
        query::Value::Object(object) => {
            for (key, obj_val) in object.clone().iter() {
                let mut new_value = obj_val.clone();
                normalize_value(&mut new_value);
                object.insert(key.clone(), new_value);
            }
        }
    }
}
