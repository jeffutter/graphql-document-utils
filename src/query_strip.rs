use crate::{query_target::Matcher, util};
use graphql_parser::{
    query::{
        parse_query, Definition as QueryDef, Directive, Document as QueryDoc, Field as QueryField,
        FragmentDefinition, InlineFragment, Selection, SelectionSet, TypeCondition, Value,
    },
    schema::parse_schema,
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
pub fn process(schema: &str, query: &str, targets: &[&str]) -> String {
    // An empty document is what this command emits when everything is stripped,
    // so it has to survive being piped back in rather than failing to parse.
    if query.trim().is_empty() {
        return String::new();
    }

    let schema_doc = parse_schema::<String>(schema).expect("Failed to parse schema");
    let query_doc = parse_query::<String>(query).expect("Failed to parse query");

    let matcher = Matcher::new(&schema_doc, targets);

    // Variables declared with a stripped type, gathered across every operation
    // before the walk starts. Fragments are shared, so a fragment has to know a
    // variable is going away without knowing which operation spread it.
    let stripped_variables: HashSet<String> = query_doc
        .definitions
        .iter()
        .filter_map(|def| match def {
            QueryDef::Operation(op) => Some(util::operation_variable_definitions(op)),
            QueryDef::Fragment(_) => None,
        })
        .flatten()
        .filter(|var| util::named_type(&var.var_type).is_some_and(|ty| matcher.is_type_target(ty)))
        .map(|var| var.name.clone())
        .collect();

    let strip = Strip {
        matcher,
        fragments: query_doc
            .definitions
            .iter()
            .filter_map(|def| match def {
                QueryDef::Fragment(f) => Some((f.name.clone(), f)),
                _ => None,
            })
            .collect(),
        stripped_variables,
    };

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
            if let Some(stripped) = strip.strip_selection_set(root_type, selection_set, &mut walk) {
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
                Some(QueryDef::Operation(util::rebuild_operation(
                    op,
                    selection_set,
                    directives,
                    &used_variables,
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
    stripped_variables: HashSet<String>,
}

impl<'s, 'q> Strip<'s, 'q, '_> {
    /// Removes every targeted selection from `selection_set` (selected on
    /// `parent_type`), returning `None` when nothing is left. `None` propagates
    /// upward: a field or inline fragment whose sub-selection empties out is
    /// itself unprintable, so it is dropped too.
    fn strip_selection_set(
        &self,
        parent_type: &str,
        selection_set: &SelectionSet<'q, String>,
        walk: &mut Walk<'q>,
    ) -> Option<SelectionSet<'q, String>> {
        let mut items = Vec::new();

        for selection in &selection_set.items {
            match selection {
                Selection::Field(field) => {
                    let field_type = self.matcher.field_type(parent_type, &field.name);

                    if self.matcher.is_field_target(parent_type, &field.name)
                        || field_type.is_some_and(|ty| self.matcher.is_type_target(ty))
                    {
                        continue;
                    }

                    let Some(field_type) = field_type else {
                        // A field the schema does not define — a meta field like
                        // `__typename`, or a query that has drifted from the
                        // schema. Its sub-selection cannot be resolved, so it is
                        // left exactly as it was rather than guessed at.
                        items.push(selection.clone());
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
                        arguments: self.retain_arguments(parent_type, field),
                        directives: self.retain_directives(&field.directives),
                        selection_set,
                        ..field.clone()
                    }));
                }
                Selection::FragmentSpread(spread) => {
                    if self.strip_fragment(&spread.fragment_name, walk) {
                        items.push(selection.clone());
                    }
                }
                Selection::InlineFragment(inline) => {
                    let condition = util::type_condition(inline.type_condition.as_ref());

                    if condition.is_some_and(|ty| self.matcher.is_type_target(ty)) {
                        continue;
                    }

                    if let Some(sub) = self.strip_selection_set(
                        condition.unwrap_or(parent_type),
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

            self.strip_selection_set(condition, &fragment.selection_set, walk)
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

    /// Drops the arguments that reference a stripped type, either by their
    /// declared type or through a variable that is going away.
    fn retain_arguments(
        &self,
        parent_type: &str,
        field: &QueryField<'q, String>,
    ) -> Vec<(String, Value<'q, String>)> {
        field
            .arguments
            .iter()
            .filter(|(name, value)| {
                let targeted_type = self
                    .matcher
                    .argument_type(parent_type, &field.name, name)
                    .is_some_and(|ty| self.matcher.is_type_target(ty));

                !targeted_type && !self.references_stripped_variable(value)
            })
            .cloned()
            .collect()
    }

    /// Drops directives fed by a stripped variable. The whole directive goes
    /// rather than just the argument, since a directive missing a required
    /// argument would not be valid.
    fn retain_directives(
        &self,
        directives: &[Directive<'q, String>],
    ) -> Vec<Directive<'q, String>> {
        directives
            .iter()
            .filter(|directive| {
                !directive
                    .arguments
                    .iter()
                    .any(|(_name, value)| self.references_stripped_variable(value))
            })
            .cloned()
            .collect()
    }

    fn references_stripped_variable(&self, value: &Value<'_, String>) -> bool {
        let mut variables = HashSet::new();
        util::collect_value_variables(value, &mut variables);
        variables
            .iter()
            .any(|name| self.stripped_variables.contains(name))
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
}
