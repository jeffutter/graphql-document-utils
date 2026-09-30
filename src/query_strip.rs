use crate::{query_target::Matcher, util};
use graphql_parser::{
    query::{
        parse_query, Definition as QueryDef, Directive, Document as QueryDoc, Field as QueryField,
        FragmentDefinition, FragmentSpread, InlineFragment, Selection, SelectionSet, TypeCondition,
        Value, VariableDefinition,
    },
    schema::{parse_schema, InputValue, Type},
};
use std::collections::{HashMap, HashSet};

/// Removes every reference to `targets` from a query, leaving the rest intact.
///
/// Each target is either a type name (`MyType`) or a field on a type
/// (`MyType.field`). This is the complement of `query focus`: where focus keeps
/// only the paths that reach a target, strip deletes exactly those selections
/// and keeps everything else.
///
/// Removal cascades. A selection set emptied by the removal cannot be printed,
/// so its parent field or inline fragment goes too, and an operation left with
/// nothing is dropped entirely. References in input position go as well:
/// arguments typed with a stripped type are deleted along with the variable
/// definitions that fed them. If everything is stripped, the result is empty.
///
/// The output stays a valid query, so an input the schema requires is never
/// removed on its own. A required field argument that has to go takes its
/// field with it (the field cannot be called without it), and the removal then
/// cascades as above. A required directive argument takes the directive
/// application instead, and a required input field inside an object literal
/// takes the whole literal, which is then removed from whatever position holds
/// it by the same rule. A list element that cannot stay is dropped on its own,
/// and a list left empty by that is removed by the same rule in turn.
///
/// Variable default values are stripped the same way, against the variable's
/// declared type. A default that only loses optional input fields keeps the
/// rest. One that loses a required input field cannot stay, and dropping it
/// would change what callers get when they omit the variable, so the variable
/// is removed as if its type had been stripped.
///
/// A field the schema does not define, such as `__typename`, is never matched
/// against the targets, since nothing says what it returns. Its arguments,
/// directives, and sub-selection are still cleaned of removed variables and of
/// fragments that did not survive, so no dangling reference is left behind.
pub fn process(schema: &str, query: &str, targets: &[&str]) -> String {
    // An empty document is what this command emits when everything is stripped,
    // so it has to survive being piped back in rather than failing to parse.
    if query.trim().is_empty() {
        return String::new();
    }

    let schema_doc = parse_schema::<String>(schema).expect("Failed to parse schema");
    let query_doc = parse_query::<String>(query).expect("Failed to parse query");

    let matcher = Matcher::new(&schema_doc, targets);

    // Variables are gathered by name across every operation before the walk
    // starts. Fragments are shared, so a fragment has to know what is happening
    // to a variable without knowing which operation spread it. Where two
    // operations declare the same name differently, the stricter outcome is
    // applied to both, which can remove more than needed but never too little.
    let variable_definitions: Vec<&VariableDefinition<String>> = query_doc
        .definitions
        .iter()
        .filter_map(|def| match def {
            QueryDef::Operation(op) => Some(util::operation_variable_definitions(op)),
            QueryDef::Fragment(_) => None,
        })
        .flatten()
        .collect();

    let mut strip = Strip {
        stripped_variables: HashSet::new(),
        matcher,
        fragments: query_doc
            .definitions
            .iter()
            .filter_map(|def| match def {
                QueryDef::Fragment(f) => Some((f.name.clone(), f)),
                _ => None,
            })
            .collect(),
    };

    // A variable goes entirely when it is declared with a stripped type, or
    // when its default value lost a required input field. Dropping just the
    // default would silently change what callers get when they omit the
    // variable, so it is treated as a reference to the stripped type instead.
    // Defaults are constant, so this does not depend on the set it feeds.
    strip.stripped_variables = variable_definitions
        .iter()
        .filter(|var| {
            util::named_type(&var.var_type).is_some_and(|ty| strip.matcher.is_type_target(ty))
                || var
                    .default_value
                    .as_ref()
                    .is_some_and(|value| strip.strip_value(Some(&var.var_type), value).is_none())
        })
        .map(|var| var.name.clone())
        .collect();

    let root_types = util::detect_root_types(&schema_doc);
    let mut walk = Walk::default();

    // Strip every operation first. Fragments are reduced on demand as their
    // spreads are reached, and the same fragment may be spread from several
    // operations, so nothing downstream can be decided until all of them have
    // been walked.
    let mut stripped_ops: HashMap<usize, SelectionSet<String>> = HashMap::new();
    for (i, def) in query_doc.definitions.iter().enumerate() {
        if let QueryDef::Operation(op) = def {
            let (root_type, selection_set) = util::operation_root(&root_types, op);
            if let Some(stripped) =
                strip.strip_selection_set(Some(root_type), selection_set, &mut walk)
            {
                stripped_ops.insert(i, stripped);
            }
        }
    }

    // Which variables each surviving operation still needs, and which fragments
    // are still spread from one. A fragment reduced during the walk may have
    // been reached only through a branch that was later dropped, so reachability
    // has to be recomputed against the surviving tree.
    let mut op_variables: HashMap<usize, HashSet<String>> = HashMap::new();
    let mut reachable_fragments: HashSet<String> = HashSet::new();
    for (i, selection_set) in &stripped_ops {
        let mut used = HashSet::new();
        strip.collect_usage(
            selection_set,
            &walk,
            &mut HashSet::new(),
            &mut used,
            &mut reachable_fragments,
        );
        used.retain(|name| !strip.stripped_variables.contains(name));
        op_variables.insert(*i, used);
    }

    let definitions: Vec<_> = query_doc
        .definitions
        .iter()
        .enumerate()
        .filter_map(|(i, def)| match def {
            QueryDef::Operation(op) => {
                let selection_set = stripped_ops.remove(&i)?;
                let directives = strip.retain_directives(util::operation_directives(op));
                let mut used_variables = op_variables.remove(&i).unwrap_or_default();
                util::collect_directive_variables(&directives, &mut used_variables);
                let variable_definitions = util::operation_variable_definitions(op)
                    .iter()
                    .filter(|var| used_variables.contains(&var.name))
                    .map(|var| VariableDefinition {
                        // Only loses optional input fields: a default that
                        // cannot be kept removed its variable above.
                        default_value: var
                            .default_value
                            .as_ref()
                            .and_then(|value| strip.strip_value(Some(&var.var_type), value)),
                        ..var.clone()
                    })
                    .collect();
                Some(QueryDef::Operation(util::rebuild_operation(
                    op,
                    selection_set,
                    directives,
                    variable_definitions,
                )))
            }
            QueryDef::Fragment(frag) if reachable_fragments.contains(&frag.name) => {
                walk.retained(&frag.name).cloned().map(QueryDef::Fragment)
            }
            QueryDef::Fragment(_) => None,
        })
        .collect();

    if definitions.is_empty() {
        return String::new();
    }

    format!("{}", QueryDoc { definitions })
}

/// Fragment bookkeeping threaded through the walk. Each fragment is reduced at
/// most once; the outcome is `None` when the fragment was stripped away.
#[derive(Default)]
struct Walk<'q> {
    outcomes: HashMap<String, Option<FragmentDefinition<'q, String>>>,
    /// Fragments currently being reduced, to break illegal fragment cycles.
    resolving: HashSet<String>,
}

impl<'q> Walk<'q> {
    fn retained(&self, name: &str) -> Option<&FragmentDefinition<'q, String>> {
        self.outcomes.get(name).and_then(|o| o.as_ref())
    }
}

struct Strip<'s, 'q, 't> {
    matcher: Matcher<'s, 't>,
    fragments: HashMap<String, &'q FragmentDefinition<'q, String>>,
    /// Variables declared with a stripped type, or whose default value cannot
    /// be kept. Every usage goes.
    stripped_variables: HashSet<String>,
}

impl<'s, 'q> Strip<'s, 'q, '_> {
    /// Removes every targeted selection from `selection_set` (selected on
    /// `parent_type`), returning `None` when nothing is left. `None` propagates
    /// upward: a field or inline fragment whose sub-selection empties out is
    /// itself unprintable, so it is dropped too.
    ///
    /// `parent_type` is `None` below a field the schema does not define: a meta
    /// field like `__typename`, or a query that has drifted from the schema.
    /// Nothing there is matched against the targets, since that would mean
    /// guessing what the field returns, until a type condition names a type
    /// again. Its inputs are still stripped, with every one counted as
    /// required, so a removed variable takes the field that passes it.
    fn strip_selection_set(
        &self,
        parent_type: Option<&str>,
        selection_set: &SelectionSet<'q, String>,
        walk: &mut Walk<'q>,
    ) -> Option<SelectionSet<'q, String>> {
        let mut items = Vec::new();

        for selection in &selection_set.items {
            match selection {
                Selection::Field(field) => {
                    let field_type =
                        parent_type.and_then(|ty| self.matcher.field_type(ty, &field.name));

                    if parent_type.is_some_and(|ty| self.matcher.is_field_target(ty, &field.name))
                        || field_type.is_some_and(|ty| self.matcher.is_type_target(ty))
                    {
                        continue;
                    }

                    // Checked before the sub-selection is walked, since a field
                    // that loses a required argument goes whatever it selects.
                    let Some(arguments) = self.strip_arguments(
                        field.arguments.iter().map(|(name, value)| (name, value)),
                        parent_type.and_then(|ty| self.matcher.field_arguments(ty, &field.name)),
                    ) else {
                        continue;
                    };

                    let selection_set = if field.selection_set.items.is_empty() {
                        field.selection_set.clone()
                    } else {
                        // A composite field whose sub-selection empties out
                        // cannot be printed, so the field goes with it.
                        match self.strip_selection_set(field_type, &field.selection_set, walk) {
                            Some(sub) => sub,
                            None => continue,
                        }
                    };

                    items.push(Selection::Field(QueryField {
                        arguments,
                        directives: self.retain_directives(&field.directives),
                        selection_set,
                        ..field.clone()
                    }));
                }
                Selection::FragmentSpread(spread) => {
                    if self.strip_fragment(&spread.fragment_name, walk) {
                        items.push(Selection::FragmentSpread(FragmentSpread {
                            directives: self.retain_directives(&spread.directives),
                            ..spread.clone()
                        }));
                    }
                }
                Selection::InlineFragment(inline) => {
                    let condition = util::type_condition(inline.type_condition.as_ref());

                    if condition.is_some_and(|ty| self.matcher.is_type_target(ty)) {
                        continue;
                    }

                    if let Some(sub) = self.strip_selection_set(
                        condition.or(parent_type),
                        &inline.selection_set,
                        walk,
                    ) {
                        items.push(Selection::InlineFragment(InlineFragment {
                            directives: self.retain_directives(&inline.directives),
                            selection_set: sub,
                            ..inline.clone()
                        }));
                    }
                }
            }
        }

        if items.is_empty() {
            None
        } else {
            Some(SelectionSet {
                span: selection_set.span,
                items,
            })
        }
    }

    /// Reduces a fragment definition, memoizing the result. Returns whether the
    /// fragment survives, which is what decides whether a spread of it is kept.
    fn strip_fragment(&self, name: &str, walk: &mut Walk<'q>) -> bool {
        if let Some(outcome) = walk.outcomes.get(name) {
            return outcome.is_some();
        }
        if !walk.resolving.insert(name.to_string()) {
            // Only reachable through an illegal fragment cycle; break it.
            return false;
        }

        let outcome = self.fragments.get(name).copied().and_then(|fragment| {
            let TypeCondition::On(condition) = &fragment.type_condition;

            // A fragment on a stripped type is a reference to it in its own
            // right, whatever it selects.
            if self.matcher.is_type_target(condition) {
                return None;
            }

            self.strip_selection_set(Some(condition), &fragment.selection_set, walk)
                .map(|selection_set| FragmentDefinition {
                    directives: self.retain_directives(&fragment.directives),
                    selection_set,
                    ..fragment.clone()
                })
        });

        walk.resolving.remove(name);
        let survives = outcome.is_some();
        walk.outcomes.insert(name.to_string(), outcome);
        survives
    }

    /// Drops the directives that cannot keep a required argument, and strips
    /// the rest of their arguments the same way as a field's. A directive the
    /// schema does not declare, such as the built-in `@include` and `@skip`,
    /// has every argument treated as required, so it goes whole.
    fn retain_directives(
        &self,
        directives: &[Directive<'q, String>],
    ) -> Vec<Directive<'q, String>> {
        directives
            .iter()
            .filter_map(|directive| {
                let arguments = self.strip_arguments(
                    directive
                        .arguments
                        .iter()
                        .map(|(name, value)| (name, value)),
                    self.matcher.directive_arguments(&directive.name),
                )?;
                Some(Directive {
                    arguments,
                    ..directive.clone()
                })
            })
            .collect()
    }

    /// Removes every reference to a stripped type from a list of arguments, or
    /// from the fields of an input object literal, given the schema's
    /// `definitions` for them. Returns `None` when one that cannot be omitted
    /// had to go, and leaves the caller to remove whatever owns the list.
    ///
    /// Only an input the schema declares nullable, or gives a default, may be
    /// removed on its own. One the schema does not describe, because it does not
    /// know the argument or its owner, is treated as required: removing too
    /// much still leaves a valid query, removing too little does not.
    fn strip_arguments<'v>(
        &self,
        arguments: impl IntoIterator<Item = (&'v String, &'v Value<'q, String>)>,
        definitions: Option<&[InputValue<'s, String>]>,
    ) -> Option<Vec<(String, Value<'q, String>)>>
    where
        'q: 'v,
    {
        let mut kept = Vec::new();
        for (name, value) in arguments {
            let definition = definitions.and_then(|defs| defs.iter().find(|def| def.name == *name));
            match self.strip_value(definition.map(|def| &def.value_type), value) {
                Some(value) => kept.push((name.clone(), value)),
                None if definition.is_some_and(|def| {
                    !matches!(def.value_type, Type::NonNullType(_)) || def.default_value.is_some()
                }) => {}
                None => return None,
            }
        }
        Some(kept)
    }

    /// What is left of `value`, supplied at an input position of type
    /// `value_type`, once every reference to a stripped type is removed.
    /// `None` when the value cannot stay at all: the position itself is typed
    /// with a stripped type, the value is a variable that is going away, or a
    /// literal nested inside it lost something it cannot do without.
    ///
    /// A list element that cannot stay is dropped and the rest keep their
    /// order. Dropping an element never breaks the list's type, whether its
    /// items are nullable or not, and a variable kept in it stays in the same
    /// item position, so its usage is still allowed. A list emptied that way
    /// cannot stay, while one written as `[]` is left alone.
    ///
    /// The position may come from the schema or, for a variable's default
    /// value, from the query, hence the separate lifetime.
    fn strip_value<'t>(
        &self,
        value_type: Option<&Type<'t, String>>,
        value: &Value<'q, String>,
    ) -> Option<Value<'q, String>> {
        let named_type = value_type.and_then(util::named_type);
        if named_type.is_some_and(|ty| self.matcher.is_type_target(ty)) {
            return None;
        }

        match value {
            Value::Variable(name) if self.stripped_variables.contains(name) => None,
            Value::List(items) => {
                let nullable = match value_type {
                    Some(Type::NonNullType(inner)) => Some(&**inner),
                    other => other,
                };
                let item_type = match nullable {
                    Some(Type::ListType(inner)) => Some(&**inner),
                    other => other,
                };
                let kept: Vec<_> = items
                    .iter()
                    .filter_map(|item| self.strip_value(item_type, item))
                    .collect();
                // A list written empty stays as it was; one emptied by the
                // strip goes, like anything else pruned to nothing.
                if kept.is_empty() && !items.is_empty() {
                    None
                } else {
                    Some(Value::List(kept))
                }
            }
            Value::Object(fields) => self
                .strip_arguments(
                    fields,
                    named_type.and_then(|ty| self.matcher.input_fields(ty)),
                )
                .map(|kept| Value::Object(kept.into_iter().collect())),
            _ => Some(value.clone()),
        }
    }

    /// Walks the surviving tree, collecting the variables it still references
    /// and the fragments still spread from it.
    fn collect_usage(
        &self,
        selection_set: &SelectionSet<'q, String>,
        walk: &Walk<'q>,
        seen_fragments: &mut HashSet<String>,
        used_variables: &mut HashSet<String>,
        reachable_fragments: &mut HashSet<String>,
    ) {
        for selection in &selection_set.items {
            match selection {
                Selection::Field(field) => {
                    for (_name, value) in &field.arguments {
                        util::collect_value_variables(value, used_variables);
                    }
                    util::collect_directive_variables(&field.directives, used_variables);
                    self.collect_usage(
                        &field.selection_set,
                        walk,
                        seen_fragments,
                        used_variables,
                        reachable_fragments,
                    );
                }
                Selection::InlineFragment(inline) => {
                    util::collect_directive_variables(&inline.directives, used_variables);
                    self.collect_usage(
                        &inline.selection_set,
                        walk,
                        seen_fragments,
                        used_variables,
                        reachable_fragments,
                    );
                }
                Selection::FragmentSpread(spread) => {
                    util::collect_directive_variables(&spread.directives, used_variables);
                    if !seen_fragments.insert(spread.fragment_name.clone()) {
                        continue;
                    }
                    if let Some(fragment) = walk.retained(&spread.fragment_name) {
                        reachable_fragments.insert(spread.fragment_name.clone());
                        util::collect_directive_variables(&fragment.directives, used_variables);
                        self.collect_usage(
                            &fragment.selection_set,
                            walk,
                            seen_fragments,
                            used_variables,
                            reachable_fragments,
                        );
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::query_strip;
    use indoc::indoc;
    use pretty_assertions::assert_eq;

    const SCHEMA: &str = indoc! {"
        schema {
          query: Query
        }

        type Query {
          user: User
          company: Company
          node(id: ID!): Node
          search(filter: SearchFilter, first: Int): [Node]
        }

        input SearchFilter {
          term: String
        }

        interface Node {
          id: ID!
        }

        type User implements Node {
          id: ID!
          name: String
          profile: Profile
          manager: User
        }

        type Profile {
          email: String
          avatar: Avatar
        }

        type Avatar {
          url: String
        }

        type Company implements Node {
          id: ID!
          name: String
          employees(first: Int): [User]
        }
    "};

    #[test]
    fn removes_fields_returning_a_targeted_type() {
        let query = indoc! {"
            query Q {
              user {
                name
                profile {
                  email
                }
              }
            }
        "};

        let result = query_strip::process(SCHEMA, query, &["Profile"]);

        assert_eq!(
            result,
            indoc! {"
                query Q {
                  user {
                    name
                  }
                }
            "}
        );
    }

    #[test]
    fn removes_only_the_targeted_field() {
        let query = indoc! {"
            query Q {
              user {
                id
                name
                profile {
                  email
                }
              }
            }
        "};

        let result = query_strip::process(SCHEMA, query, &["User.name"]);

        assert_eq!(
            result,
            indoc! {"
                query Q {
                  user {
                    id
                    profile {
                      email
                    }
                  }
                }
            "}
        );
    }

    /// Removing the last selection of a field takes the field with it, and so on
    /// up the tree until something survives.
    #[test]
    fn cascades_removal_through_emptied_parents() {
        let query = indoc! {"
            query Q {
              user {
                profile {
                  avatar {
                    url
                  }
                }
              }
              company {
                name
              }
            }
        "};

        let result = query_strip::process(SCHEMA, query, &["Avatar.url"]);

        assert_eq!(
            result,
            indoc! {"
                query Q {
                  company {
                    name
                  }
                }
            "}
        );
    }

    #[test]
    fn removes_matching_inline_fragments() {
        let query = indoc! {"
            query Q {
              node(id: \"1\") {
                id
                ... on User {
                  name
                }
                ... on Company {
                  name
                }
              }
            }
        "};

        let result = query_strip::process(SCHEMA, query, &["User"]);

        assert_eq!(
            result,
            indoc! {"
                query Q {
                  node(id: \"1\") {
                    id
                    ... on Company {
                      name
                    }
                  }
                }
            "}
        );
    }

    #[test]
    fn strips_fragments_in_place_and_drops_emptied_ones() {
        let query = indoc! {"
            query Q {
              user {
                ...UserFields
              }
              company {
                ...CompanyFields
              }
            }

            fragment UserFields on User {
              id
              name
            }

            fragment CompanyFields on Company {
              name
            }
        "};

        let result = query_strip::process(SCHEMA, query, &["Node.id", "Company.name"]);

        assert_eq!(
            result,
            indoc! {"
                query Q {
                  user {
                    ...UserFields
                  }
                }

                fragment UserFields on User {
                  name
                }
            "}
        );
    }

    /// A fragment whose type condition is targeted is a reference to that type
    /// however little of it the fragment selects.
    #[test]
    fn drops_fragments_defined_on_a_targeted_type() {
        let query = indoc! {"
            query Q {
              node(id: \"1\") {
                id
                ...UserFields
              }
            }

            fragment UserFields on User {
              name
            }
        "};

        let result = query_strip::process(SCHEMA, query, &["User"]);

        assert_eq!(
            result,
            indoc! {"
                query Q {
                  node(id: \"1\") {
                    id
                  }
                }
            "}
        );
    }

    /// Subtype-aware matching, mirroring `query focus`: stripping an interface
    /// strips its implementors too.
    #[test]
    fn strips_implementors_of_a_targeted_interface() {
        let query = indoc! {"
            query Q {
              user {
                name
              }
              company {
                name
              }
            }
        "};

        assert_eq!(query_strip::process(SCHEMA, query, &["Node"]), "");
    }

    #[test]
    fn removes_arguments_and_variables_typed_with_a_targeted_input() {
        let query = indoc! {"
            query Q($filter: SearchFilter, $first: Int) {
              search(filter: $filter, first: $first) {
                id
              }
            }
        "};

        let result = query_strip::process(SCHEMA, query, &["SearchFilter"]);

        assert_eq!(
            result,
            indoc! {"
                query Q($first: Int) {
                  search(first: $first) {
                    id
                  }
                }
            "}
        );
    }

    #[test]
    fn drops_variables_left_unreferenced_by_the_removal() {
        let query = indoc! {"
            query Q($id: ID!, $first: Int) {
              node(id: $id) {
                id
              }
              company {
                employees(first: $first) {
                  name
                }
              }
            }
        "};

        let result = query_strip::process(SCHEMA, query, &["User"]);

        assert_eq!(
            result,
            indoc! {"
                query Q($id: ID!) {
                  node(id: $id) {
                    id
                  }
                }
            "}
        );
    }

    #[test]
    fn drops_operations_left_with_nothing() {
        let query = indoc! {"
            query OnlyProfile {
              user {
                profile {
                  email
                }
              }
            }

            query AlsoName {
              user {
                name
                profile {
                  email
                }
              }
            }
        "};

        let result = query_strip::process(SCHEMA, query, &["Profile"]);

        assert_eq!(
            result,
            indoc! {"
                query AlsoName {
                  user {
                    name
                  }
                }
            "}
        );
    }

    /// A fragment reduced while walking a branch that was later dropped must not
    /// be emitted: nothing spreads it any more.
    #[test]
    fn drops_fragments_no_surviving_operation_spreads() {
        let query = indoc! {"
            query Q {
              user {
                profile {
                  ...ProfileFields
                }
              }
              company {
                name
              }
            }

            fragment ProfileFields on Profile {
              email
            }
        "};

        let result = query_strip::process(SCHEMA, query, &["Profile.email"]);

        assert_eq!(
            result,
            indoc! {"
                query Q {
                  company {
                    name
                  }
                }
            "}
        );
    }

    #[test]
    fn keeps_meta_fields_the_schema_does_not_define() {
        let query = indoc! {"
            query Q {
              user {
                __typename
                name
                profile {
                  email
                }
              }
            }
        "};

        let result = query_strip::process(SCHEMA, query, &["Profile"]);

        assert_eq!(
            result,
            indoc! {"
                query Q {
                  user {
                    __typename
                    name
                  }
                }
            "}
        );
    }

    #[test]
    fn returns_empty_when_everything_is_stripped() {
        let query = indoc! {"
            query Q {
              user {
                name
              }
            }
        "};

        assert_eq!(query_strip::process(SCHEMA, query, &["User"]), "");
    }

    /// The empty document this command emits when everything is stripped has to
    /// survive being piped back in.
    #[test]
    fn passes_an_empty_document_through() {
        assert_eq!(query_strip::process(SCHEMA, "", &["User"]), "");
        assert_eq!(query_strip::process(SCHEMA, "\n", &["User"]), "");
    }

    #[test]
    fn leaves_a_query_untouched_when_nothing_matches() {
        let query = indoc! {"
            query Q {
              user {
                name
              }
            }
        "};

        let result = query_strip::process(SCHEMA, query, &["NoSuchType", "User.missing"]);

        assert_eq!(result, query);
    }

    #[test]
    fn resolves_fields_and_arguments_added_by_extensions() {
        let schema = indoc! {"
            type Query {
              me: User
            }

            extend type Query {
              bots(filter: BotFilter, first: Int): [Bot]
            }

            input BotFilter {
              model: String
            }

            type User {
              id: ID
              bot: Bot
            }

            type Bot {
              id: ID
            }

            extend type Bot {
              model: String
            }
        "};
        let query = indoc! {"
            query($f: BotFilter) {
              bots(filter: $f, first: 2) {
                id
                model
              }
              me {
                id
              }
            }
        "};

        // `bots` exists only in an extension, so stripping `Bot` must see it.
        let result = query_strip::process(schema, query, &["Bot"]);
        assert_eq!(
            result,
            indoc! {"
                query {
                  me {
                    id
                  }
                }
            "}
        );

        // So do its argument types and the fields the `Bot` extension adds.
        let result = query_strip::process(schema, query, &["BotFilter", "Bot.model"]);
        assert_eq!(
            result,
            indoc! {"
                query {
                  bots(first: 2) {
                    id
                  }
                  me {
                    id
                  }
                }
            "}
        );
    }

    /// A schema whose inputs are required at every level: field arguments,
    /// input object fields, and directive arguments.
    const REQUIRED_SCHEMA: &str = indoc! {"
        directive @auth(role: Role!, sort: Sort) on FIELD | FRAGMENT_SPREAD

        type Query {
          search(filter: SearchFilter!, first: Int): [Item]
          items(filter: SearchFilter, sort: Sort! = NAME): [Item]
          pinned(filter: SearchFilter! = {status: ACTIVE}): [Item]
          batch(filters: [SearchFilter!]): [Item]
          me: User
        }

        input SearchFilter {
          term: String
          status: Status!
          sort: Sort
        }

        enum Status {
          ACTIVE
          ARCHIVED
        }

        enum Sort {
          NAME
          DATE
        }

        enum Role {
          ADMIN
          USER
        }

        type Item {
          id: ID
          name: String
        }

        type User {
          id: ID
          name: String
          items(filter: SearchFilter!): [Item]
        }
    "};

    /// The field cannot be called without its required argument, so it goes
    /// instead, and the operation it empties goes with it.
    #[test]
    fn removes_the_field_when_a_required_argument_is_stripped() {
        let query = indoc! {"
            query($f: SearchFilter!) {
              search(filter: $f, first: 2) {
                id
              }
            }
        "};

        assert_eq!(
            query_strip::process(REQUIRED_SCHEMA, query, &["SearchFilter"]),
            ""
        );
    }

    #[test]
    fn keeps_siblings_of_a_field_removed_for_a_required_argument() {
        let query = indoc! {"
            query Q($f: SearchFilter!, $n: Int) {
              search(filter: $f, first: $n) {
                id
              }
              me {
                id
              }
            }
        "};

        let result = query_strip::process(REQUIRED_SCHEMA, query, &["SearchFilter"]);

        // `$n` fed only the removed field, so it goes too.
        assert_eq!(
            result,
            indoc! {"
                query Q {
                  me {
                    id
                  }
                }
            "}
        );
    }

    #[test]
    fn cascades_a_required_argument_removal_to_the_operation() {
        let query = indoc! {"
            query OnlyItems($f: SearchFilter!) {
              me {
                items(filter: $f) {
                  id
                }
              }
            }

            query AlsoName($f: SearchFilter!) {
              me {
                name
                items(filter: $f) {
                  id
                }
              }
            }
        "};

        let result = query_strip::process(REQUIRED_SCHEMA, query, &["SearchFilter"]);

        assert_eq!(
            result,
            indoc! {"
                query AlsoName {
                  me {
                    name
                  }
                }
            "}
        );
    }

    #[test]
    fn removes_a_nullable_argument_on_its_own() {
        let query = indoc! {"
            query($f: SearchFilter) {
              items(filter: $f) {
                id
              }
            }
        "};

        let result = query_strip::process(REQUIRED_SCHEMA, query, &["SearchFilter"]);

        assert_eq!(
            result,
            indoc! {"
                query {
                  items {
                    id
                  }
                }
            "}
        );
    }

    /// `sort` is non-null, but the schema default fills it in when it is left
    /// out, so it can go on its own.
    #[test]
    fn removes_an_argument_with_a_schema_default_on_its_own() {
        let query = indoc! {"
            query {
              items(sort: DATE) {
                id
              }
            }
        "};

        let result = query_strip::process(REQUIRED_SCHEMA, query, &["Sort"]);

        assert_eq!(
            result,
            indoc! {"
                query {
                  items {
                    id
                  }
                }
            "}
        );
    }

    /// Requiredness is decided at each level of an input literal: an optional
    /// input field goes on its own, while a required one takes the literal, and
    /// then the literal's own position decides what goes with it.
    #[test]
    fn strips_input_object_literals_level_by_level() {
        // `sort` is an optional input field, so only it is removed.
        let query = indoc! {"
            query {
              search(filter: {term: \"x\", sort: DATE, status: ACTIVE}) {
                id
              }
            }
        "};
        assert_eq!(
            query_strip::process(REQUIRED_SCHEMA, query, &["Sort"]),
            indoc! {"
                query {
                  search(filter: {status: ACTIVE, term: \"x\"}) {
                    id
                  }
                }
            "}
        );

        // `status` is a required input field, so the literal cannot stay. Under
        // the required `search(filter:)` that takes the field; under the
        // nullable `items(filter:)` only the argument goes. A stripped variable
        // in a required input field counts the same as a literal.
        let query = indoc! {"
            query($s: Status!) {
              search(filter: {term: \"x\", status: ACTIVE}) {
                id
              }
              other: search(filter: {status: $s}) {
                id
              }
              items(filter: {term: \"x\", status: ACTIVE}) {
                id
              }
            }
        "};
        assert_eq!(
            query_strip::process(REQUIRED_SCHEMA, query, &["Status"]),
            indoc! {"
                query {
                  items {
                    id
                  }
                }
            "}
        );

        // List elements are stripped the same way. One that only loses an
        // optional input field stays, reduced, in its place; one that loses a
        // required input field is dropped.
        let query = indoc! {"
            query {
              batch(filters: [{status: ACTIVE, sort: DATE}, {status: ARCHIVED}]) {
                id
              }
            }
        "};
        assert_eq!(
            query_strip::process(REQUIRED_SCHEMA, query, &["Sort"]),
            indoc! {"
                query {
                  batch(filters: [{status: ACTIVE}, {status: ARCHIVED}]) {
                    id
                  }
                }
            "}
        );
        let query = indoc! {"
            query {
              batch(filters: [{status: ACTIVE, term: \"a\"}, {status: ARCHIVED}]) {
                id
              }
            }
        "};
        assert_eq!(
            query_strip::process(REQUIRED_SCHEMA, query, &["Status"]),
            indoc! {"
                query {
                  batch {
                    id
                  }
                }
            "}
        );
    }

    /// A directive that would lose a required argument is dropped, while an
    /// optional one goes on its own and the directive stays.
    #[test]
    fn drops_a_directive_that_loses_a_required_argument() {
        let query = indoc! {"
            query {
              me @auth(role: ADMIN, sort: NAME) {
                id
              }
            }
        "};

        assert_eq!(
            query_strip::process(REQUIRED_SCHEMA, query, &["Sort"]),
            indoc! {"
                query {
                  me @auth(role: ADMIN) {
                    id
                  }
                }
            "}
        );
        assert_eq!(
            query_strip::process(REQUIRED_SCHEMA, query, &["Role"]),
            indoc! {"
                query {
                  me {
                    id
                  }
                }
            "}
        );
    }

    /// Inside a fragment the field removal cascades as anywhere else: a
    /// fragment that keeps something stays, one that empties takes its spreads.
    /// Directives on the spreads themselves are stripped too.
    #[test]
    fn removes_fields_with_required_arguments_inside_fragments() {
        let query = indoc! {"
            query($f: SearchFilter!) {
              me {
                ...UserFields @auth(role: ADMIN)
                ...UserItems
              }
            }

            fragment UserFields on User {
              id
              items(filter: {status: ACTIVE}) {
                id
              }
            }

            fragment UserItems on User {
              items(filter: $f) {
                name
              }
            }
        "};

        let result = query_strip::process(REQUIRED_SCHEMA, query, &["SearchFilter", "Role"]);

        assert_eq!(
            result,
            indoc! {"
                query {
                  me {
                    ...UserFields
                  }
                }

                fragment UserFields on User {
                  id
                }
            "}
        );
    }

    /// A field the schema does not define is never matched against the targets,
    /// but its inputs still are. Every one counts as required, since nothing
    /// says otherwise, so a removed variable in any argument takes the field,
    /// and a directive on it is stripped as it would be anywhere else.
    #[test]
    fn strips_inputs_of_fields_the_schema_does_not_define() {
        let query = indoc! {"
            query($f: SearchFilter, $r: Role!, $n: Int) {
              me {
                id
                __typename @auth(role: $r)
              }
              unknown(x: $f, first: $n) {
                id
              }
              nested(x: {y: [$f]}) {
                id
              }
              legacy(first: $n) @auth(role: ADMIN) {
                id
              }
            }
        "};

        let result = query_strip::process(REQUIRED_SCHEMA, query, &["SearchFilter", "Role"]);

        assert_eq!(
            result,
            indoc! {"
                query($n: Int) {
                  me {
                    id
                    __typename
                  }
                  legacy(first: $n) {
                    id
                  }
                }
            "}
        );
    }

    /// Below a field the schema does not define nothing is matched against the
    /// targets, but removed variables and fragments are still cleaned out, and
    /// the removal cascades up through the unknown field as usual. A type
    /// condition names its type, so matching resumes inside it.
    #[test]
    fn strips_the_sub_selection_of_fields_the_schema_does_not_define() {
        let query = indoc! {"
            query($f: SearchFilter) {
              me {
                id
              }
              drifted {
                deep(y: [$f])
              }
              spreads {
                ...Items
                ...Names
              }
              typed {
                ... on Item {
                  id
                }
              }
            }

            fragment Items on Item {
              id
            }

            fragment Names on User {
              name
            }
        "};

        let result = query_strip::process(REQUIRED_SCHEMA, query, &["SearchFilter", "Item"]);

        // `Items` is on a stripped type, so its spread goes rather than being
        // left pointing at a fragment that is no longer emitted.
        assert_eq!(
            result,
            indoc! {"
                query {
                  me {
                    id
                  }
                  spreads {
                    ...Names
                  }
                }

                fragment Names on User {
                  name
                }
            "}
        );
    }

    /// Variable defaults are stripped level by level like any other literal,
    /// against the variable's declared type. A variable whose own type is
    /// stripped goes whole, default and all.
    #[test]
    fn strips_variable_default_values_level_by_level() {
        let query = indoc! {"
            query($f: SearchFilter = {term: \"x\", sort: DATE, status: ACTIVE}, $s: Sort = NAME) {
              items(filter: $f, sort: $s) {
                id
              }
            }
        "};

        assert_eq!(
            query_strip::process(REQUIRED_SCHEMA, query, &["Sort"]),
            indoc! {"
                query($f: SearchFilter = {status: ACTIVE, term: \"x\"}) {
                  items(filter: $f) {
                    id
                  }
                }
            "}
        );
    }

    /// A default that loses a required input field cannot stay, and dropping
    /// just the default would change what callers get when they omit the
    /// variable. So the variable is removed as if its type had been stripped,
    /// nullable or not, and every usage cascades as a removed variable does: a
    /// required argument takes its field, a nullable argument or one with a
    /// schema default goes on its own, and a list element is dropped from its
    /// list.
    #[test]
    fn removes_a_variable_whose_default_loses_a_required_input_field() {
        let query = indoc! {"
            query(
              $f: SearchFilter = {status: ACTIVE, term: \"x\"}
              $n: SearchFilter! = {status: ACTIVE}
              $l: SearchFilter = {status: ARCHIVED}
              $k: SearchFilter!
            ) {
              search(filter: $f) {
                id
              }
              items(filter: $f) {
                id
              }
              pinned(filter: $f) {
                id
              }
              other: search(filter: $n) {
                id
              }
              batch(filters: [$l, $k]) {
                id
              }
            }
        "};

        let result = query_strip::process(REQUIRED_SCHEMA, query, &["Status"]);

        assert_eq!(
            result,
            indoc! {"
                query($k: SearchFilter!) {
                  items {
                    id
                  }
                  pinned {
                    id
                  }
                  batch(filters: [$k]) {
                    id
                  }
                }
            "}
        );
    }

    /// Variables are tracked by name, since fragments are shared. A variable
    /// removed for its default in one operation is removed from every operation
    /// that declares the name, so the shared fragment agrees with all of them.
    #[test]
    fn removes_a_variable_whose_default_cannot_be_kept_inside_fragments() {
        let query = indoc! {"
            query A($f: SearchFilter = {status: ACTIVE}) {
              me {
                id
                ...UserItems
              }
            }

            query B($f: SearchFilter!) {
              me {
                name
                ...UserItems
              }
            }

            fragment UserItems on User {
              items(filter: $f) {
                id
              }
            }
        "};

        let result = query_strip::process(REQUIRED_SCHEMA, query, &["Status"]);

        assert_eq!(
            result,
            indoc! {"
                query A {
                  me {
                    id
                  }
                }

                query B {
                  me {
                    name
                  }
                }
            "}
        );
    }

    const DEFAULT_SCHEMA: &str = indoc! {"
        type Query {
          search(filter: SearchFilter): [Item]
          pinned(filter: SearchFilter!): [Item]
        }

        input SearchFilter {
          status: Status!
          term: String
        }

        enum Status {
          ACTIVE
          ARCHIVED
        }

        type Item {
          id: ID
        }
    "};

    /// A non-null variable whose default loses a required input field is
    /// removed rather than left without a default, which would make it
    /// required. The nullable argument it fed goes on its own.
    #[test]
    fn removes_a_non_null_variable_whose_default_cannot_be_kept() {
        let query = indoc! {"
            query($f: SearchFilter! = {status: ACTIVE, term: \"x\"}) {
              search(filter: $f) {
                id
              }
            }
        "};

        let result = query_strip::process(DEFAULT_SCHEMA, query, &["Status"]);

        assert_eq!(
            result,
            indoc! {"
                query {
                  search {
                    id
                  }
                }
            "}
        );
    }

    /// A nullable variable whose default loses a required input field is
    /// removed rather than left to default to null. Each usage then goes by its
    /// position: the nullable argument on its own, the required one with its
    /// field.
    #[test]
    fn removes_a_nullable_variable_whose_default_cannot_be_kept() {
        let query = indoc! {"
            query($f: SearchFilter = {status: ACTIVE}) {
              search(filter: $f) {
                id
              }
              pinned(filter: $f) {
                id
              }
            }
        "};

        let result = query_strip::process(DEFAULT_SCHEMA, query, &["Status"]);

        assert_eq!(
            result,
            indoc! {"
                query {
                  search {
                    id
                  }
                }
            "}
        );
    }

    /// A default that loses only optional input fields keeps the rest, and the
    /// variable and its usages stay.
    #[test]
    fn keeps_a_default_that_loses_only_optional_input_fields() {
        let query = indoc! {"
            query($f: SearchFilter = {status: ACTIVE, term: \"x\"}) {
              search(filter: $f) {
                id
              }
              pinned(filter: $f) {
                id
              }
            }
        "};

        let result = query_strip::process(DEFAULT_SCHEMA, query, &["String"]);

        assert_eq!(
            result,
            indoc! {"
                query($f: SearchFilter = {status: ACTIVE}) {
                  search(filter: $f) {
                    id
                  }
                  pinned(filter: $f) {
                    id
                  }
                }
            "}
        );
    }

    /// List positions with every combination of list and item nullability.
    const LIST_SCHEMA: &str = indoc! {"
        type Query {
          search(filters: [F!], first: Int): [Item]
          loose(filters: [F]): [Item]
          required(filters: [F!]!, first: Int): [Item]
          nested(filters: [[F!]]): [Item]
          statuses(in: [Status!]): [Item]
          me: Item
        }

        input F {
          status: Status!
          term: String
        }

        enum Status {
          ACTIVE
          ARCHIVED
        }

        type Item {
          id: ID
        }
    "};

    /// A list element that cannot stay is dropped, and the rest keep their
    /// order. Item nullability makes no difference: the element goes rather
    /// than being replaced with null.
    #[test]
    fn drops_only_the_list_elements_that_cannot_stay() {
        let query = indoc! {"
            query {
              search(filters: [{status: ACTIVE}, {term: \"x\"}], first: 2) {
                id
              }
              loose(filters: [{term: \"a\"}, {status: ACTIVE}, {term: \"b\"}]) {
                id
              }
            }
        "};

        let result = query_strip::process(LIST_SCHEMA, query, &["Status"]);

        assert_eq!(
            result,
            indoc! {"
                query {
                  search(filters: [{term: \"x\"}], first: 2) {
                    id
                  }
                  loose(filters: [{term: \"a\"}, {term: \"b\"}]) {
                    id
                  }
                }
            "}
        );
    }

    /// A list emptied by the strip cannot stay, and its position then decides
    /// as usual: a nullable argument goes on its own, a required one takes its
    /// field, and the removal cascades from there.
    #[test]
    fn removes_a_list_emptied_by_the_strip() {
        let query = indoc! {"
            query {
              search(filters: [{status: ACTIVE}], first: 2) {
                id
              }
              required(filters: [{status: ACTIVE}], first: 2) {
                id
              }
            }

            query OnlyRequired {
              required(filters: [{status: ARCHIVED}]) {
                id
              }
            }
        "};

        let result = query_strip::process(LIST_SCHEMA, query, &["Status"]);

        assert_eq!(
            result,
            indoc! {"
                query {
                  search(first: 2) {
                    id
                  }
                }
            "}
        );
    }

    /// A list written empty is left as it is, at any depth. Only a list emptied
    /// by the strip is removed.
    #[test]
    fn leaves_a_list_written_empty_alone() {
        let query = indoc! {"
            query {
              search(filters: []) {
                id
              }
              nested(filters: [[], [{status: ACTIVE}]]) {
                id
              }
            }
        "};

        let result = query_strip::process(LIST_SCHEMA, query, &["Status"]);

        assert_eq!(
            result,
            indoc! {"
                query {
                  search(filters: []) {
                    id
                  }
                  nested(filters: [[]]) {
                    id
                  }
                }
            "}
        );
    }

    /// Nested lists are stripped level by level: an inner list emptied by the
    /// strip is an element that cannot stay, so it is dropped from the outer
    /// list, and an outer list emptied that way goes too.
    #[test]
    fn strips_nested_lists_level_by_level() {
        let query = indoc! {"
            query {
              nested(filters: [[{status: ACTIVE}], [{term: \"x\"}, {status: ARCHIVED}]]) {
                id
              }
              other: nested(filters: [[{status: ACTIVE}]]) {
                id
              }
            }
        "};

        let result = query_strip::process(LIST_SCHEMA, query, &["Status"]);

        assert_eq!(
            result,
            indoc! {"
                query {
                  nested(filters: [[{term: \"x\"}]]) {
                    id
                  }
                  other: nested {
                    id
                  }
                }
            "}
        );
    }

    /// A removed variable in a list is dropped like any other element. The
    /// variable kept beside it stays in the same item position, so its usage
    /// is still allowed.
    #[test]
    fn drops_a_removed_variable_from_a_list() {
        let query = indoc! {"
            query($a: F!, $removed: F = {status: ACTIVE}) {
              search(filters: [$a, $removed]) {
                id
              }
            }
        "};

        let result = query_strip::process(LIST_SCHEMA, query, &["Status"]);

        assert_eq!(
            result,
            indoc! {"
                query($a: F!) {
                  search(filters: [$a]) {
                    id
                  }
                }
            "}
        );
    }

    /// A list default loses the elements that cannot stay and keeps the rest.
    /// One left empty cannot stay, so its variable is removed like one whose
    /// default lost a required input field.
    #[test]
    fn strips_list_variable_defaults_element_by_element() {
        let query = indoc! {"
            query($f: [F!] = [{status: ACTIVE}, {term: \"x\"}], $g: [F!] = [{status: ACTIVE}]) {
              search(filters: $f) {
                id
              }
              other: search(filters: $g, first: 1) {
                id
              }
            }
        "};

        let result = query_strip::process(LIST_SCHEMA, query, &["Status"]);

        assert_eq!(
            result,
            indoc! {"
                query($f: [F!] = [{term: \"x\"}]) {
                  search(filters: $f) {
                    id
                  }
                  other: search(first: 1) {
                    id
                  }
                }
            "}
        );
    }

    /// Enum values in a list belong to a stripped enum only when the list's
    /// item type is that enum, so the position itself is stripped and the list
    /// goes whole, before any element is looked at.
    #[test]
    fn removes_a_list_of_a_stripped_enum_whole() {
        let query = indoc! {"
            query {
              statuses(in: [ACTIVE, ARCHIVED]) {
                id
              }
              me {
                id
              }
            }
        "};

        let result = query_strip::process(LIST_SCHEMA, query, &["Status"]);

        assert_eq!(
            result,
            indoc! {"
                query {
                  statuses {
                    id
                  }
                  me {
                    id
                  }
                }
            "}
        );
    }
}
