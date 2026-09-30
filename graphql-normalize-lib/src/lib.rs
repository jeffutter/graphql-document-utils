// The README is the crate's documentation, on crates.io and docs.rs alike, so
// its example runs as a doc test.
#![doc = include_str!("../README.md")]

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

        // Ranked by kind, then by name, so no name can sort a definition into
        // another kind's place. Anonymous operations have the empty name.
        self.0.definitions.sort_by_key(|d| match d {
            Definition::Operation(o) => match o {
                query::OperationDefinition::SelectionSet(_) => (0, String::new()),
                query::OperationDefinition::Query(q) => (0, lowercase(q.name.as_deref())),
                query::OperationDefinition::Mutation(m) => (1, lowercase(m.name.as_deref())),
                query::OperationDefinition::Subscription(s) => (2, lowercase(s.name.as_deref())),
            },
            Definition::Fragment(frag) => (3, frag.name.to_lowercase()),
        });
    }
}

/// `name` lowercased, or the empty string for none.
fn lowercase(name: Option<&str>) -> String {
    name.map(str::to_lowercase).unwrap_or_default()
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

    // Fields, then spreads, then inline fragments, each by name, ranked by
    // kind first so no name can sort a selection into another kind's place.
    selections.sort_by_key(|s| match s {
        Selection::Field(f) => (0, f.name.to_lowercase()),
        Selection::FragmentSpread(fs) => (1, fs.fragment_name.to_lowercase()),
        Selection::InlineFragment(f) => match &f.type_condition {
            Some(query::TypeCondition::On(on)) => (2, on.to_lowercase()),
            None => (2, String::new()),
        },
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

    #[test]
    fn orders_operations_by_type_then_name_then_fragments_by_name() {
        let query = "fragment Z on T { a } fragment a on T { a } subscription S { a } mutation M { a } query b { a } query A { a }";

        let expected = indoc! {"
            query A {
              a
            }

            query b {
              a
            }

            mutation M {
              a
            }

            subscription S {
              a
            }

            fragment a on T {
              a
            }

            fragment Z on T {
              a
            }
        "};

        assert_eq!(normalize(query).unwrap(), expected);
    }

    /// Fields by name, not alias, then spreads, then inline fragments by type
    /// condition, whatever the names: a field named `zzzzb` once sorted after
    /// the spreads, whose sort key was prefixed with `zzzz`.
    #[test]
    fn orders_fields_then_spreads_then_inline_fragments() {
        let query = "{ ... on B { x } ...Y z: a ...x zzzzb ... on a { x } b }";

        let expected = indoc! {"
            {
              z: a
              b
              zzzzb
              ...x
              ...Y
              ... on a {
                x
              }
              ... on B {
                x
              }
            }
        "};

        assert_eq!(normalize(query).unwrap(), expected);
    }

    /// Names compare without regard to case, and ones that compare equal,
    /// such as a field selected under two aliases, keep their order. Input
    /// object fields are the exception: the parser keeps them in a `BTreeMap`,
    /// which sorts them as written.
    #[test]
    fn sorts_arguments_and_directives_by_name_regardless_of_case() {
        let query = "{ f(b: 1, C: 2, o: {b: 1, C: 2}) @b(y: 1, X: 2) @A y: g x: g }";

        let expected = indoc! {"
            {
              f(b: 1, C: 2, o: {C: 2, b: 1}) @A @b(X: 2, y: 1)
              y: g
              x: g
            }
        "};

        assert_eq!(normalize(query).unwrap(), expected);
    }

    #[test]
    fn drops_comments() {
        let query = "# a query\n{ a # the first\n b }";

        assert_eq!(normalize(query).unwrap(), "{\n  a\n  b\n}\n");
    }
}
