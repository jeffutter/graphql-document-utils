use graphql_parser::query::{self, Definition, Directive, Document, Selection, VariableDefinition};
use std::fmt::Display;

/// Parses a GraphQL query document and prints it back in a canonical order.
///
/// Definitions, selections, directives, variable definitions, and argument
/// names are sorted. Argument values are left as written: list items keep
/// their order, since that order is meaningful (`orderBy: [...]`, positional
/// inputs). Input object fields come out sorted by key wherever they appear,
/// because the parser stores them in a `BTreeMap`.
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
        directive.arguments.sort_by_key(|(k, _v)| k.to_lowercase());
    }

    directives.sort_by_key(|d| d.name.to_lowercase());
}

fn normalize_variable_definitions(variable_definitions: &mut [VariableDefinition<String>]) {
    variable_definitions.sort_by_key(|vd| vd.name.to_lowercase());
}

#[cfg(test)]
mod tests {
    use super::*;
    use indoc::indoc;
    use pretty_assertions::assert_eq;

    #[test]
    fn preserves_int_list_order() {
        let query = "query ($o: [Int] = [10, 2, 1]) { me @x(list: [10, 2, 1]) { id } }";

        let expected = indoc! {"
            query($o: [Int] = [10, 2, 1]) {
              me @x(list: [10, 2, 1]) {
                id
              }
            }
        "};

        assert_eq!(normalize(query).unwrap(), expected);
    }

    #[test]
    fn preserves_enum_and_string_list_order() {
        let query = indoc! {r#"
            query ($sort: [Sort] = [NAME_DESC, AGE_ASC]) {
              users(orderBy: [NAME_DESC, AGE_ASC], ids: ["b", "a", "c"]) @x(tags: ["z", "y"]) {
                id
              }
            }
        "#};

        let expected = indoc! {r#"
            query($sort: [Sort] = [NAME_DESC, AGE_ASC]) {
              users(ids: ["b", "a", "c"], orderBy: [NAME_DESC, AGE_ASC]) @x(tags: ["z", "y"]) {
                id
              }
            }
        "#};

        assert_eq!(normalize(query).unwrap(), expected);
    }

    #[test]
    fn sorts_keys_of_objects_inside_lists_without_reordering_the_list() {
        let query = indoc! {"
            query ($o: [Order] = [{field: NAME, dir: DESC}, {field: AGE, dir: ASC}]) {
              users(orderBy: [{field: NAME, dir: DESC}, {field: AGE, dir: ASC}]) {
                id
              }
            }
        "};

        let expected = indoc! {"
            query($o: [Order] = [{dir: DESC, field: NAME}, {dir: ASC, field: AGE}]) {
              users(orderBy: [{dir: DESC, field: NAME}, {dir: ASC, field: AGE}]) {
                id
              }
            }
        "};

        assert_eq!(normalize(query).unwrap(), expected);
    }

    #[test]
    fn normalizes_field_argument_values_like_directive_arguments() {
        let query = indoc! {"
            query {
              users(where: {z: [3, 1, 2], a: {y: 1, b: [{d: 1, c: 2}]}}) @x(where: {z: [3, 1, 2], a: {y: 1, b: [{d: 1, c: 2}]}}) {
                id
              }
            }
        "};

        let expected = indoc! {"
            query {
              users(where: {a: {b: [{c: 2, d: 1}], y: 1}, z: [3, 1, 2]}) @x(where: {a: {b: [{c: 2, d: 1}], y: 1}, z: [3, 1, 2]}) {
                id
              }
            }
        "};

        assert_eq!(normalize(query).unwrap(), expected);
    }
}
