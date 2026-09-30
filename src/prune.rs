use crate::{
    error::Error,
    input::Input,
    supergraph::{enum_argument, string_argument, FieldSet, Graph, Supergraph},
    util, Output,
};
use graphql_parser::{
    query::{Definition as QueryDef, FragmentDefinition, Selection, SelectionSet, TypeCondition},
    schema::{Field, InterfaceType, ObjectType, TypeDefinition, UnionType},
};
use std::collections::{HashMap, HashSet};

/// Processes the schema and query files to prune unused types and fields.
///
/// Object and interface types keep the fields the query selects on them or on
/// an interface they implement. Every type the query enters is kept, as is
/// every object implementing an interface it enters, since such an object can
/// be returned in the interface's place. Unions keep only those members.
/// Everything the kept definitions depend on is kept too (see
/// `util::retain_with_dependencies`), along with the definitions of directives
/// the query applies.
///
/// A definition trimmed down to empty is removed, along with every reference
/// to it, such as an interface none of whose fields the query selects, which
/// kept types then stop implementing. A type a kept field returns is never
/// removed, since the query or an implementor would no longer be valid against
/// the output: one the query selects nothing but `__typename` on, or one an
/// implementor narrows an interface field to and the query never enters,
/// keeps its smallest valid form (see `complete_used_fields`).
///
/// The schema's query root type is always kept, since a schema without one is
/// invalid. When the query never enters it, as in a document of only
/// mutations, it keeps its smallest valid form too.
///
/// A blank schema passes through as empty, as it does through every schema
/// command. A blank query, as `query strip` emits once it has removed
/// everything, uses nothing, so the output is empty too, and a note says why,
/// since the schema given was not. That is not the smallest valid schema a
/// query of only mutations gets: a blank query is no query at all, not one
/// that happens to leave the query root unentered, and inventing a root field
/// would hide that nothing reached the prune. Both documents are parsed before
/// either is found blank, so a syntax error in the other is still reported.
///
/// A supergraph (see `Supergraph::detect`) stays one a router can load. Its
/// machinery is kept whole (see `util::retain_with_dependencies`), and the
/// fields its join directives name are used like the query's own, as are the
/// fields each subgraph needs to keep what it resolves (see
/// `complete_used_fields`). A field set that does not parse is
/// `Error::MalformedSupergraph`.
///
/// A name either document defines more than once adds a warning to
/// `warnings`, and only its first definition is kept (see
/// `Input::parse_schema`/`parse_query`), as does what a supergraph records that
/// pruning does not follow (see `Supergraph::untracked`).
pub fn process(schema: &Input, query: &Input, warnings: &mut Vec<String>) -> Result<Output, Error> {
    let (schema_doc, query_doc) =
        match (schema.parse_schema(warnings)?, query.parse_query(warnings)?) {
            (Some(schema_doc), Some(query_doc)) => (schema_doc, query_doc),
            (None, _) => return Ok(Output::default()),
            (Some(_), None) => {
                return Ok(Output {
                    document: String::new(),
                    notes: vec!["the query is empty; output is empty".to_string()],
                })
            }
        };

    // Extensions are merged in, so fields and interfaces an extension adds are
    // seen as used like any other, and `retain_with_dependencies` trims each
    // `extend` block to its share of what survives.
    let type_map = util::merged_type_definitions(&schema_doc);

    let federation = match Supergraph::detect(&schema_doc) {
        Some(supergraph) => {
            warnings.extend(supergraph.untracked(&type_map));
            let field_sets =
                supergraph
                    .field_sets(&type_map)
                    .map_err(|message| Error::MalformedSupergraph {
                        origin: schema.origin().clone(),
                        message,
                    })?;
            let graphs = supergraph.graphs(&type_map);
            Some(Federation {
                supergraph,
                field_sets,
                graphs,
            })
        }
        None => None,
    };

    let fragments: HashMap<_, _> = query_doc
        .definitions
        .iter()
        .filter_map(|def| {
            if let QueryDef::Fragment(f) = def {
                Some((f.name.clone(), f))
            } else {
                None
            }
        })
        .collect();

    let root_types = util::detect_root_types(&schema_doc);

    let mut used_fields: HashMap<String, HashSet<String>> = HashMap::new();
    let mut used_directives: HashSet<String> = HashSet::new();
    let mut variable_types: HashSet<String> = HashSet::new();

    for def in &query_doc.definitions {
        match def {
            QueryDef::Operation(op) => {
                let (op_type, selection_set) = util::operation_root(&root_types, op);
                collect_used_fields(
                    op_type,
                    selection_set,
                    &type_map,
                    &mut used_fields,
                    &fragments,
                );

                used_directives.extend(
                    util::operation_directives(op)
                        .iter()
                        .map(|directive| directive.name.clone()),
                );
                collect_selection_directives(selection_set, &mut used_directives);
                variable_types.extend(
                    util::operation_variable_definitions(op)
                        .iter()
                        .filter_map(|var| util::named_type(&var.var_type).cloned()),
                );
            }
            QueryDef::Fragment(frag) => {
                used_directives.extend(frag.directives.iter().map(|d| d.name.clone()));
                collect_selection_directives(&frag.selection_set, &mut used_directives);
            }
        }
    }

    // A schema must have a query root type, so a query document of only
    // mutations or subscriptions still enters it, and it keeps its smallest
    // valid form (see `complete_used_fields`). A schema without one is left
    // without one.
    if matches!(
        type_map.get(&root_types.query),
        Some(TypeDefinition::Object(_))
    ) {
        used_fields.entry(root_types.query.clone()).or_default();
    }

    complete_used_fields(&type_map, federation.as_ref(), &mut used_fields);
    let entered = entered_types(&type_map, &used_fields);

    let seed_types: Vec<&str> = entered
        .iter()
        .copied()
        .chain(variable_types.iter().map(String::as_str))
        .collect();
    let seed_directives: Vec<&str> = used_directives.iter().map(String::as_str).collect();

    let pruned_doc =
        util::retain_with_dependencies(&schema_doc, &seed_types, &seed_directives, |td| match td {
            TypeDefinition::Object(obj) => TypeDefinition::Object(ObjectType {
                fields: obj
                    .fields
                    .iter()
                    .filter(|f| keeps_field(&used_fields, &obj.name, &obj.implements_interfaces, f))
                    .cloned()
                    .collect(),
                ..obj.clone()
            }),
            TypeDefinition::Interface(iface) => TypeDefinition::Interface(InterfaceType {
                fields: iface
                    .fields
                    .iter()
                    .filter(|f| {
                        keeps_field(&used_fields, &iface.name, &iface.implements_interfaces, f)
                    })
                    .cloned()
                    .collect(),
                ..iface.clone()
            }),
            TypeDefinition::Union(union) => TypeDefinition::Union(UnionType {
                types: union
                    .types
                    .iter()
                    .filter(|member| entered.contains(member.as_str()))
                    .cloned()
                    .collect(),
                ..union.clone()
            }),
            TypeDefinition::Scalar(_)
            | TypeDefinition::Enum(_)
            | TypeDefinition::InputObject(_) => td.clone(),
        });

    Ok(pruned_doc.to_string().into())
}

/// The types the query enters, plus every object implementing an interface it
/// enters, since such an object can be returned in the interface's place.
fn entered_types<'t>(
    type_map: &'t HashMap<String, TypeDefinition<'_, String>>,
    used_fields: &HashMap<String, HashSet<String>>,
) -> HashSet<&'t str> {
    type_map
        .iter()
        .filter(|(name, td)| match td {
            TypeDefinition::Object(obj) => {
                used_fields.contains_key(*name)
                    || obj
                        .implements_interfaces
                        .iter()
                        .any(|i| used_fields.contains_key(i))
            }
            _ => used_fields.contains_key(*name),
        })
        .map(|(name, _)| name.as_str())
        .collect()
}

/// Whether `field` of the named type survives: it is selected on the type or on
/// one of its `interfaces`. An implementor must declare every field of its
/// interfaces, so a field selected through an interface is kept on the types
/// implementing it.
fn keeps_field(
    used_fields: &HashMap<String, HashSet<String>>,
    type_name: &str,
    interfaces: &[String],
    field: &Field<'_, String>,
) -> bool {
    let selected = |name: &str| {
        used_fields
            .get(name)
            .is_some_and(|set| set.contains(&field.name))
    };
    selected(type_name) || interfaces.iter().any(|iface| selected(iface))
}

/// What a supergraph adds to what pruning must keep.
struct Federation {
    supergraph: Supergraph,
    field_sets: Vec<FieldSet>,
    graphs: Vec<Graph>,
}

/// Adds to `used_fields` what the output needs beyond what the query selects,
/// so no type it keeps is trimmed to empty and removed. Each removal would
/// cascade to a field returning the type, leaving the query or an implementor
/// invalid against the output. The rules apply until none adds anything.
///
/// Every kept field enters the type it returns. For a field the query selects
/// that is already so, but an implementor keeps every field its interfaces
/// keep, and may narrow one's type to a subtype: `f: Member` for the
/// interface's `f: SomeUnion`, or an object for an interface, under any list
/// and non-null wrappers. The implementor must still declare `f`, so the
/// narrower type is entered as well. Entering a union member also keeps it in
/// the union, which the narrowing needs. Interfaces a kept type implements are
/// followed too, since they are kept without being entered, and one
/// implementing another can narrow in turn.
///
/// Every entered type left empty is treated as though the query had selected
/// the least that keeps it valid. That happens when the query selects nothing
/// on a type but `__typename`, as in `search { __typename }` or
/// `... on Node { __typename }`, or never enters a type narrowed to as above.
/// An object or interface selects one field: the first that returns a scalar
/// or enum and takes no arguments, falling back to the first that returns one,
/// then the first that takes none, then the first. A union selects its first
/// member, which then needs a field of its own, as may the type the selected
/// field returns. Unions go first, then interfaces, those implementing fewer
/// interfaces first, since what an interface selects carries over to its
/// implementors, which then need nothing of their own.
///
/// A supergraph adds two rules, applied before the last since they may leave
/// nothing empty. First, the router selects what the join directives name, so
/// every kept type uses its keys (for every subgraph), and every kept field
/// the fields its `requires` names on the type and its `provides` on the type
/// it returns, as a query selection would, entering the types they reach. The
/// type a kept field has in a subgraph (`@join__field(type:)`) is entered too.
/// Second, each subgraph must stay valid on its own: a kept field a subgraph
/// resolves returns a type that subgraph must keep something of, so an object
/// or interface keeping no field the subgraph resolves selects the one it
/// would above among those it does, and a union keeping no member the
/// subgraph has selects the first it has.
///
/// Types already empty in the source have nothing to select and are left as
/// written.
fn complete_used_fields(
    type_map: &HashMap<String, TypeDefinition<'_, String>>,
    federation: Option<&Federation>,
    used_fields: &mut HashMap<String, HashSet<String>>,
) {
    let composite = |name: &String| {
        matches!(
            type_map.get(name),
            Some(
                TypeDefinition::Object(_) | TypeDefinition::Interface(_) | TypeDefinition::Union(_)
            )
        )
    };
    let order = |td: &TypeDefinition<'_, String>| match td {
        TypeDefinition::Union(_) => (0, 0),
        TypeDefinition::Interface(iface) => (1, iface.implements_interfaces.len()),
        _ => (2, 0),
    };

    // Each type is settled at most once, which bounds the loop even for a
    // union whose members the schema does not define as objects.
    let mut settled: HashSet<String> = HashSet::new();
    loop {
        // Enter the return type of every field a kept type keeps. The kept
        // objects and interfaces are the entered ones and every interface they
        // implement, directly or through another interface.
        loop {
            let mut kept: Vec<&str> = entered_types(type_map, used_fields).into_iter().collect();
            let mut seen: HashSet<&str> = kept.iter().copied().collect();
            let mut entering: Vec<String> = Vec::new();
            let mut applying: Vec<&FieldSet> = Vec::new();
            while let Some(name) = kept.pop() {
                let (implements_interfaces, fields) = match type_map.get(name) {
                    Some(TypeDefinition::Object(ObjectType {
                        implements_interfaces,
                        fields,
                        ..
                    }))
                    | Some(TypeDefinition::Interface(InterfaceType {
                        implements_interfaces,
                        fields,
                        ..
                    })) => (implements_interfaces, fields),
                    _ => continue,
                };
                kept.extend(
                    implements_interfaces
                        .iter()
                        .map(String::as_str)
                        .filter(|iface| seen.insert(iface)),
                );
                let kept_fields = fields
                    .iter()
                    .filter(|f| keeps_field(used_fields, name, implements_interfaces, f));
                for field in kept_fields {
                    let graph_types = federation
                        .map(|fed| fed.supergraph.graph_types(&field.directives))
                        .unwrap_or_default();
                    entering.extend(
                        util::named_type(&field.field_type)
                            .cloned()
                            .into_iter()
                            .chain(graph_types)
                            .filter(|returns| {
                                composite(returns) && !used_fields.contains_key(returns)
                            }),
                    );
                }
                if let Some(fed) = federation {
                    applying.extend(fed.field_sets.iter().filter(|fs| {
                        fs.type_name == name
                            && fs.field.as_ref().is_none_or(|field| {
                                fields.iter().any(|f| {
                                    f.name == *field
                                        && keeps_field(used_fields, name, implements_interfaces, f)
                                })
                            })
                    }));
                }
            }
            let before = used_count(used_fields);
            for name in entering {
                used_fields.entry(name).or_default();
            }
            for fs in applying {
                collect_used_fields(
                    &fs.on,
                    &fs.selection,
                    type_map,
                    used_fields,
                    &HashMap::new(),
                );
            }
            if used_count(used_fields) == before {
                break;
            }
        }

        if let Some(fed) = federation {
            if complete_graphs(type_map, fed, used_fields) {
                continue;
            }
        }

        let entered = entered_types(type_map, used_fields);
        let is_empty = |td: &TypeDefinition<'_, String>| match td {
            TypeDefinition::Object(ObjectType {
                name,
                implements_interfaces,
                fields,
                ..
            })
            | TypeDefinition::Interface(InterfaceType {
                name,
                implements_interfaces,
                fields,
                ..
            }) => !fields
                .iter()
                .any(|f| keeps_field(used_fields, name, implements_interfaces, f)),
            TypeDefinition::Union(union) => {
                !union.types.iter().any(|m| entered.contains(m.as_str()))
            }
            TypeDefinition::Scalar(_)
            | TypeDefinition::Enum(_)
            | TypeDefinition::InputObject(_) => false,
        };
        let Some((name, td)) = used_fields
            .keys()
            .filter(|name| !settled.contains(*name))
            .filter_map(|name| type_map.get_key_value(name))
            .filter(|(_, td)| is_empty(td))
            .min_by_key(|(name, td)| (order(td), *name))
        else {
            break;
        };
        settled.insert(name.clone());

        // The type the selected field returns is entered by the next pass.
        match td {
            TypeDefinition::Object(ObjectType { fields, .. })
            | TypeDefinition::Interface(InterfaceType { fields, .. }) => {
                if let Some(field) = util::smallest_field(type_map, fields) {
                    used_fields
                        .entry(name.clone())
                        .or_default()
                        .insert(field.name.clone());
                }
            }
            TypeDefinition::Union(union) => {
                if let Some(member) = union
                    .types
                    .iter()
                    .find(|member| matches!(type_map.get(*member), Some(TypeDefinition::Object(_))))
                {
                    used_fields.entry(member.clone()).or_default();
                }
            }
            _ => (),
        }
    }
}

/// How many types are entered and fields used, which only grows, so a pass
/// that leaves it unchanged added nothing.
fn used_count(used_fields: &HashMap<String, HashSet<String>>) -> usize {
    used_fields.len() + used_fields.values().map(HashSet::len).sum::<usize>()
}

/// Applies the second supergraph rule of `complete_used_fields` once: for
/// each kept field, each subgraph resolving it must keep something of the type
/// the field returns there. Returns whether it added anything.
fn complete_graphs(
    type_map: &HashMap<String, TypeDefinition<'_, String>>,
    fed: &Federation,
    used_fields: &mut HashMap<String, HashSet<String>>,
) -> bool {
    let sg = &fed.supergraph;
    let entered = entered_types(type_map, used_fields);
    let in_graph = |td: &TypeDefinition<'_, String>, graph: &str| {
        sg.type_graphs(td, &fed.graphs).iter().any(|g| g == graph)
    };

    // The kept objects and interfaces, and the interfaces they implement.
    let mut kept: Vec<&str> = entered.iter().copied().collect();
    let mut seen: HashSet<&str> = kept.iter().copied().collect();
    let mut adding: Vec<(String, Option<String>)> = Vec::new();
    while let Some(name) = kept.pop() {
        let Some(td @ (TypeDefinition::Object(_) | TypeDefinition::Interface(_))) =
            type_map.get(name)
        else {
            continue;
        };
        let (implements_interfaces, fields) = match td {
            TypeDefinition::Object(obj) => (&obj.implements_interfaces, &obj.fields),
            TypeDefinition::Interface(iface) => (&iface.implements_interfaces, &iface.fields),
            _ => unreachable!(),
        };
        kept.extend(
            implements_interfaces
                .iter()
                .map(String::as_str)
                .filter(|iface| seen.insert(iface)),
        );
        let owner_in = |graph: &str| in_graph(td, graph);
        for field in fields
            .iter()
            .filter(|f| keeps_field(used_fields, name, implements_interfaces, f))
        {
            for graph in &fed.graphs {
                let graph = graph.value.as_str();
                if !sg.resolves_in(&field.directives, owner_in(graph), graph) {
                    continue;
                }
                let returns = sg.type_in(field, graph);
                let Some(returns) = util::named_type(&returns) else {
                    continue;
                };
                let Some(target) = type_map.get(returns).filter(|t| in_graph(t, graph)) else {
                    continue;
                };
                match target {
                    TypeDefinition::Object(ObjectType {
                        implements_interfaces,
                        fields,
                        ..
                    })
                    | TypeDefinition::Interface(InterfaceType {
                        implements_interfaces,
                        fields,
                        ..
                    }) => {
                        let mut resolved = fields
                            .iter()
                            .filter(|f| sg.resolves_in(&f.directives, true, graph));
                        if resolved
                            .clone()
                            .any(|f| keeps_field(used_fields, returns, implements_interfaces, f))
                        {
                            continue;
                        }
                        if let Some(field) = util::smallest_field(type_map, &mut resolved) {
                            adding.push((returns.clone(), Some(field.name.clone())));
                        }
                    }
                    TypeDefinition::Union(union) => {
                        let listed = sg.joins(&union.directives, "unionMember");
                        let members: Vec<&String> = union
                            .types
                            .iter()
                            .filter(|member| {
                                listed.is_empty()
                                    || listed.iter().any(|d| {
                                        enum_argument(d, "graph") == Some(graph)
                                            && string_argument(d, "member") == Some(member.as_str())
                                    })
                            })
                            .filter(|member| {
                                matches!(
                                    type_map.get(*member),
                                    Some(t @ TypeDefinition::Object(_)) if in_graph(t, graph)
                                )
                            })
                            .collect();
                        if !members.iter().any(|m| entered.contains(m.as_str())) {
                            if let Some(first) = members.first() {
                                adding.push(((*first).clone(), None));
                            }
                        }
                    }
                    TypeDefinition::Scalar(_)
                    | TypeDefinition::Enum(_)
                    | TypeDefinition::InputObject(_) => (),
                }
            }
        }
    }

    let before = used_count(used_fields);
    for (type_name, field) in adding {
        let used = used_fields.entry(type_name).or_default();
        used.extend(field);
    }
    used_count(used_fields) != before
}

/// Collects used fields from the selection set, and marks `parent_type` and
/// every type the selection set reaches as entered, even when nothing but
/// `__typename` is selected on it.
fn collect_used_fields<'a>(
    parent_type: &str,
    selection_set: &SelectionSet<String>,
    type_map: &HashMap<String, TypeDefinition<'a, String>>,
    used_fields: &mut HashMap<String, HashSet<String>>,
    fragments: &HashMap<String, &'a FragmentDefinition<'a, String>>,
) {
    if let Some(parent_def) = type_map.get(parent_type) {
        used_fields.entry(parent_type.to_string()).or_default();
        let fields = util::type_fields(parent_def);

        for selection in &selection_set.items {
            match selection {
                Selection::Field(field) => {
                    if let Some(schema_field) =
                        fields.and_then(|fields| fields.iter().find(|f| f.name == field.name))
                    {
                        used_fields
                            .entry(parent_type.to_string())
                            .or_default()
                            .insert(field.name.clone());

                        collect_used_fields(
                            util::named_type(&schema_field.field_type).unwrap(),
                            &field.selection_set,
                            type_map,
                            used_fields,
                            fragments,
                        );
                    }
                }
                Selection::FragmentSpread(spread) => {
                    if let Some(frag) = fragments.get(&spread.fragment_name) {
                        let TypeCondition::On(type_condition) = &frag.type_condition;
                        collect_used_fields(
                            type_condition,
                            &frag.selection_set,
                            type_map,
                            used_fields,
                            fragments,
                        );
                    }
                }
                Selection::InlineFragment(frag) => {
                    let type_name = frag
                        .type_condition
                        .clone()
                        .map(|tc| match tc {
                            TypeCondition::On(name) => name,
                        })
                        .unwrap_or(parent_type.to_string());

                    collect_used_fields(
                        &type_name,
                        &frag.selection_set,
                        type_map,
                        used_fields,
                        fragments,
                    );
                }
            }
        }
    }
}

/// Collects the name of every directive applied within a selection set. The
/// schema must keep their definitions for the query to stay valid against it.
fn collect_selection_directives(selection_set: &SelectionSet<String>, used: &mut HashSet<String>) {
    for selection in &selection_set.items {
        let (directives, nested) = match selection {
            Selection::Field(field) => (&field.directives, Some(&field.selection_set)),
            Selection::FragmentSpread(spread) => (&spread.directives, None),
            Selection::InlineFragment(frag) => (&frag.directives, Some(&frag.selection_set)),
        };
        used.extend(directives.iter().map(|directive| directive.name.clone()));
        if let Some(nested) = nested {
            collect_selection_directives(nested, used);
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{input::Input, prune, supergraph::Supergraph, util};
    use graphql_parser::{
        parse_query, parse_schema,
        query::{Definition, FragmentDefinition, Selection, SelectionSet, TypeCondition},
        schema::{Definition as SchemaDef, TypeDefinition},
    };
    use indoc::indoc;
    use pretty_assertions::assert_eq;
    use std::collections::HashMap;

    /// Prunes and checks that the output names only types and directives it
    /// defines, has no empty definitions, still satisfies every interface it
    /// keeps, still defines everything the query uses, and keeps the query root
    /// type when the input schema has one. A supergraph must stay one
    /// (`util::assert_valid_supergraph`).
    fn pruned(schema: &str, query: &str) -> String {
        let result = prune::process(
            &Input::inline(schema),
            &Input::inline(query),
            &mut Vec::new(),
        )
        .unwrap()
        .document;
        util::assert_self_contained(&result);
        util::assert_no_empty_definitions(&result);
        assert_implementors_complete(&result);
        assert_supports_query(&result, query);
        if let Some(root) = query_root(schema) {
            assert_has_query_root(&result, &root);
        }
        if is_supergraph(schema) {
            util::assert_valid_supergraph(&result);
        }
        result
    }

    fn is_supergraph(schema: &str) -> bool {
        Supergraph::detect(&parse_schema::<String>(schema).unwrap()).is_some()
    }

    /// The query root type `schema` defines, resolved as pruning resolves it,
    /// or `None` when no object type of that name is defined.
    fn query_root(schema: &str) -> Option<String> {
        let doc = parse_schema::<String>(schema).unwrap();
        let root = util::detect_root_types(&doc).query;
        let types = util::merged_type_definitions(&doc);
        matches!(types.get(&root), Some(TypeDefinition::Object(_))).then_some(root)
    }

    /// Asserts that `schema` defines `root` as an object type and, if it has a
    /// schema definition, names `root` there as the query root.
    fn assert_has_query_root(schema: &str, root: &str) {
        let doc = parse_schema::<String>(schema).unwrap();
        let types = util::merged_type_definitions(&doc);
        assert!(
            matches!(types.get(root), Some(TypeDefinition::Object(_))),
            "schema lacks its query root type {root}:\n{schema}"
        );
        for def in &doc.definitions {
            if let SchemaDef::SchemaDefinition(sd) = def {
                assert_eq!(
                    sd.query.as_deref(),
                    Some(root),
                    "schema definition does not name {root} as the query root:\n{schema}"
                );
            }
        }
    }

    /// Asserts that every object or interface in `schema` declares every field
    /// of each interface it implements, returning the same named type or a
    /// subtype of it: an implementor of the interface, or a member of the
    /// union. Wrappers are not compared, since pruning keeps fields whole.
    fn assert_implementors_complete(schema: &str) {
        fn implements<'r>(td: &'r TypeDefinition<'_, String>) -> Option<&'r Vec<String>> {
            match td {
                TypeDefinition::Object(obj) => Some(&obj.implements_interfaces),
                TypeDefinition::Interface(iface) => Some(&iface.implements_interfaces),
                _ => None,
            }
        }

        let doc = parse_schema::<String>(schema).unwrap();
        let types = util::merged_type_definitions(&doc);
        let is_subtype = |sub: &String, sup: &String| {
            sub == sup
                || match (types.get(sub), types.get(sup)) {
                    (Some(sub_td), Some(TypeDefinition::Interface(_))) => {
                        implements(sub_td).is_some_and(|ifaces| ifaces.contains(sup))
                    }
                    (_, Some(TypeDefinition::Union(union))) => union.types.contains(sub),
                    _ => false,
                }
        };
        for (name, td) in &types {
            let (Some(interfaces), Some(fields)) = (implements(td), util::type_fields(td)) else {
                continue;
            };
            for iface in interfaces {
                let iface_fields = types.get(iface).and_then(util::type_fields).unwrap();
                for expected in iface_fields {
                    let field = fields
                        .iter()
                        .find(|f| f.name == expected.name)
                        .unwrap_or_else(|| {
                            panic!("{name} lacks {iface}.{}:\n{schema}", expected.name)
                        });
                    let returns = util::named_type(&field.field_type).unwrap();
                    let expected_returns = util::named_type(&expected.field_type).unwrap();
                    assert!(
                        is_subtype(returns, expected_returns),
                        "{name}.{} returns {returns}, not a subtype of {iface}.{}'s \
                         {expected_returns}:\n{schema}",
                        field.name,
                        expected.name,
                    );
                }
            }
        }
    }

    /// Asserts that every type, field, and variable type `query` names is
    /// defined in `schema`, so the query is still valid against it as far as
    /// names go.
    fn assert_supports_query(schema: &str, query: &str) {
        type Types<'a> = HashMap<String, TypeDefinition<'a, String>>;
        type Fragments<'r, 'q> = HashMap<&'r str, &'r FragmentDefinition<'q, String>>;

        fn check<'r, 'q>(
            type_name: &str,
            selection_set: &'r SelectionSet<'q, String>,
            types: &Types,
            fragments: &Fragments<'r, 'q>,
            schema: &str,
        ) {
            let td = types
                .get(type_name)
                .unwrap_or_else(|| panic!("query enters undefined type {type_name}:\n{schema}"));
            for selection in &selection_set.items {
                let (type_name, selection_set) = match selection {
                    Selection::Field(field) if field.name == "__typename" => continue,
                    Selection::Field(field) => {
                        let definition = util::type_fields(td)
                            .and_then(|fields| fields.iter().find(|f| f.name == field.name))
                            .unwrap_or_else(|| {
                                panic!(
                                    "query selects undefined field {type_name}.{}:\n{schema}",
                                    field.name
                                )
                            });
                        if field.selection_set.items.is_empty() {
                            continue;
                        }
                        let returns = util::named_type(&definition.field_type).unwrap();
                        (returns.as_str(), &field.selection_set)
                    }
                    Selection::FragmentSpread(spread) => {
                        let fragment = fragments[spread.fragment_name.as_str()];
                        let TypeCondition::On(condition) = &fragment.type_condition;
                        (condition.as_str(), &fragment.selection_set)
                    }
                    Selection::InlineFragment(inline) => (
                        util::type_condition(inline.type_condition.as_ref()).unwrap_or(type_name),
                        &inline.selection_set,
                    ),
                };
                check(type_name, selection_set, types, fragments, schema);
            }
        }

        const BUILT_IN_TYPES: [&str; 5] = ["Boolean", "Float", "ID", "Int", "String"];
        let schema_doc = parse_schema::<String>(schema).unwrap();
        let query_doc = parse_query::<String>(query).unwrap();
        let types = util::merged_type_definitions(&schema_doc);
        let root_types = util::detect_root_types(&schema_doc);
        let fragments: Fragments = query_doc
            .definitions
            .iter()
            .filter_map(|def| match def {
                Definition::Fragment(fragment) => Some((fragment.name.as_str(), fragment)),
                Definition::Operation(_) => None,
            })
            .collect();
        for def in &query_doc.definitions {
            let Definition::Operation(op) = def else {
                continue;
            };
            let (root, selection_set) = util::operation_root(&root_types, op);
            check(root, selection_set, &types, &fragments, schema);
            for variable in util::operation_variable_definitions(op) {
                let name = util::named_type(&variable.var_type).unwrap();
                assert!(
                    types.contains_key(name) || BUILT_IN_TYPES.contains(&name.as_str()),
                    "query declares a variable of undefined type {name}:\n{schema}"
                );
            }
        }
    }

    #[test]
    fn prunes_fields() {
        let schema = indoc! {"
            type Query {
              user: User
            }

            type User {
              id: ID!
              name: String
              first_name: String
              last_name: String
            }
        "};

        let query = indoc! {"
            query User {
              user {
                id
                name
              }
            }
        "};

        let result = pruned(schema, query);

        assert_eq!(
            result,
            indoc! {"
                type Query {
                  user: User
                }

                type User {
                  id: ID!
                  name: String
                }
            "}
        );
    }

    fn pruned_output(schema: &str, query: &str) -> (String, Vec<String>) {
        let output = prune::process(
            &Input::inline(schema),
            &Input::inline(query),
            &mut Vec::new(),
        )
        .unwrap();
        (output.document, output.notes)
    }

    const USER_SCHEMA: &str = "type Query { user: User }\ntype User { id: ID }\n";

    /// A blank query, as `query strip` leaves once it removes everything, uses
    /// nothing, so the schema prunes to nothing, with a note saying why.
    #[test]
    fn prunes_to_nothing_with_a_blank_query() {
        for query in ["", "\n", "# stripped\n"] {
            assert_eq!(
                pruned_output(USER_SCHEMA, query),
                (
                    String::new(),
                    vec!["the query is empty; output is empty".to_string()]
                ),
                "{query:?}"
            );
        }
    }

    /// A blank schema passes through as empty, silently, as it does through
    /// every schema command, whatever the query.
    #[test]
    fn passes_a_blank_schema_through() {
        for query in ["{ user { id } }", ""] {
            assert_eq!(
                pruned_output("\n", query),
                (String::new(), Vec::new()),
                "{query:?}"
            );
        }
    }

    /// Blank or not, each document is parsed, so a syntax error in either is
    /// reported whatever the other holds.
    #[test]
    fn reports_a_syntax_error_beside_a_blank_document() {
        let error = |schema, query| {
            prune::process(
                &Input::inline(schema),
                &Input::inline(query),
                &mut Vec::new(),
            )
            .expect_err("expected the prune to fail")
            .to_string()
        };
        assert_eq!(
            error("", "{ user { id }"),
            "failed to parse query (stdin) at 1:14: unexpected end of input; expected }"
        );
        assert_eq!(
            error("type Query {", ""),
            "failed to parse schema (stdin) at 1:13: unexpected end of input; expected Name"
        );
    }

    #[test]
    fn prunes_interface_fields() {
        let schema = indoc! {"
            type Query {
              person: Person
            }

            interface Person {
              id: ID!
              name: String
              first_name: String
              last_name: String
            }

            type User implements Person {
              id: ID!
              name: String
              first_name: String
              last_name: String
              user_name: String
              login: String
            }

            type Customer implements Person {
              id: ID!
              name: String
              first_name: String
              last_name: String
              last_visited: String
            }

            type Guest implements Person {
              id: ID!
              name: String
              first_name: String
              last_name: String
            }
        "};

        let query = indoc! {"
            query Person {
              person {
                id
                name
                ... on User {
                  user_name
                }
              }
            }
        "};

        let result = pruned(schema, query);

        assert_eq!(
            result,
            indoc! {"
                type Query {
                  person: Person
                }

                interface Person {
                  id: ID!
                  name: String
                }

                type User implements Person {
                  id: ID!
                  name: String
                  user_name: String
                }

                type Customer implements Person {
                  id: ID!
                  name: String
                }

                type Guest implements Person {
                  id: ID!
                  name: String
                }
            "}
        );
    }

    #[test]
    fn keeps_fields_when_fragment_spread_revisits_type() {
        let schema = indoc! {"
            type User {
              id: ID!
              name: String
              profile: Profile
            }

            type Profile {
              bio: String
            }

            type Query {
              me: User
            }
        "};

        let query = indoc! {"
            {
              me {
                profile {
                  bio
                }
                ...B
              }
            }

            fragment B on User {
              name
            }
        "};

        let result = pruned(schema, query);

        assert_eq!(
            result,
            indoc! {"
                type User {
                  name: String
                  profile: Profile
                }

                type Profile {
                  bio: String
                }

                type Query {
                  me: User
                }
            "}
        );
    }

    #[test]
    fn keeps_fields_when_inline_fragment_revisits_type() {
        let schema = indoc! {"
            type User {
              id: ID!
              name: String
              profile: Profile
            }

            type Profile {
              bio: String
            }

            type Query {
              me: User
            }
        "};

        let query = indoc! {"
            {
              me {
                profile {
                  bio
                }
                ... on User {
                  name
                }
              }
            }
        "};

        let result = pruned(schema, query);

        assert_eq!(
            result,
            indoc! {"
                type User {
                  name: String
                  profile: Profile
                }

                type Profile {
                  bio: String
                }

                type Query {
                  me: User
                }
            "}
        );
    }

    #[test]
    fn keeps_fields_from_every_operation_on_same_root() {
        let schema = indoc! {"
            type User {
              id: ID!
              name: String
            }

            type Query {
              me: User
              version: String
            }
        "};

        let query = indoc! {"
            query A {
              me {
                id
              }
            }

            query B {
              version
            }
        "};

        let result = pruned(schema, query);

        assert_eq!(
            result,
            indoc! {"
                type User {
                  id: ID!
                }

                type Query {
                  me: User
                  version: String
                }
            "}
        );
    }

    #[test]
    fn keeps_types_named_by_kept_fields() {
        let schema = indoc! {"
            scalar DateTime

            enum Status {
              ACTIVE
              INACTIVE
            }

            interface Node {
              id: ID!
            }

            input F {
              name: String
            }

            type User implements Node {
              id: ID!
              name: String
              created: DateTime
              status: Status
            }

            type Query {
              me: User
              search(filter: F): [User]
            }
        "};

        // No Node field is selected, so Node is trimmed to empty and goes, and
        // User stops implementing it.
        let query = indoc! {"
            {
              me {
                created
                status
              }
            }
        "};

        let result = pruned(schema, query);

        assert_eq!(
            result,
            indoc! {"
                scalar DateTime

                enum Status {
                  ACTIVE
                  INACTIVE
                }

                type User {
                  created: DateTime
                  status: Status
                }

                type Query {
                  me: User
                }
            "}
        );
    }

    #[test]
    fn keeps_argument_input_types_recursively() {
        let schema = indoc! {"
            enum Order {
              ASC
              DESC
            }

            scalar Cursor

            input Page {
              after: Cursor
              order: [Order!]
            }

            input Filter {
              name: String
              page: Page
              and: [Filter!]
            }

            input Unused {
              name: String
            }

            type User {
              id: ID!
            }

            type Query {
              search(filter: Filter!): [User]
              other(unused: Unused): User
            }
        "};

        let query = indoc! {"
            query ($filter: Filter!) {
              search(filter: $filter) {
                id
              }
            }
        "};

        let result = pruned(schema, query);

        assert_eq!(
            result,
            indoc! {"
                enum Order {
                  ASC
                  DESC
                }

                scalar Cursor

                input Page {
                  after: Cursor
                  order: [Order!]
                }

                input Filter {
                  name: String
                  page: Page
                  and: [Filter!]
                }

                type User {
                  id: ID!
                }

                type Query {
                  search(filter: Filter!): [User]
                }
            "}
        );
    }

    #[test]
    fn keeps_union_members_the_query_enters() {
        let schema = indoc! {"
            type User {
              id: ID!
              name: String
            }

            type Post {
              title: String
            }

            type Comment {
              body: String
            }

            union SearchResult = User | Post | Comment

            type Query {
              search: [SearchResult]
            }
        "};

        let query = indoc! {"
            {
              search {
                ... on User {
                  name
                }
                ... on Post {
                  title
                }
              }
            }
        "};

        let result = pruned(schema, query);

        assert_eq!(
            result,
            indoc! {"
                type User {
                  name: String
                }

                type Post {
                  title: String
                }

                union SearchResult = User | Post

                type Query {
                  search: [SearchResult]
                }
            "}
        );
    }

    #[test]
    fn keeps_one_member_of_a_union_the_query_enters_none_of() {
        let schema = indoc! {"
            type User {
              id: ID!
            }

            type Post {
              title: String
            }

            union SearchResult = User | Post

            type Query {
              search: [SearchResult]
            }
        "};

        // The query needs SearchResult, which would be empty with no member
        // entered, so it keeps its first member, and that member its first
        // field, as though the query had selected them.
        let query = indoc! {"
            {
              search {
                __typename
              }
            }
        "};

        let result = pruned(schema, query);

        assert_eq!(
            result,
            indoc! {"
                type User {
                  id: ID!
                }

                union SearchResult = User

                type Query {
                  search: [SearchResult]
                }
            "}
        );
    }

    #[test]
    fn keeps_only_directive_definitions_in_use() {
        let schema = indoc! {"
            directive @auth(requires: Role) on FIELD_DEFINITION

            directive @tag(name: String) on ENUM_VALUE | INPUT_FIELD_DEFINITION | ARGUMENT_DEFINITION

            directive @cached(ttl: Int) on FIELD

            directive @unused on FIELD_DEFINITION

            directive @dropped on FIELD_DEFINITION

            enum Role {
              ADMIN
              USER
            }

            enum Status {
              ACTIVE @tag(name: \"a\")
            }

            input Filter {
              status: Status @tag(name: \"s\")
            }

            type Query {
              me(filter: Filter @tag(name: \"f\")): String @auth(requires: ADMIN)
              other: String @dropped
            }
        "};

        let query = indoc! {"
            {
              me @cached(ttl: 10)
            }
        "};

        let result = pruned(schema, query);

        assert_eq!(
            result,
            indoc! {"
                directive @auth(requires: Role) on FIELD_DEFINITION

                directive @tag(name: String) on ENUM_VALUE | INPUT_FIELD_DEFINITION | ARGUMENT_DEFINITION

                directive @cached(ttl: Int) on FIELD

                enum Role {
                  ADMIN
                  USER
                }

                enum Status {
                  ACTIVE @tag(name: \"a\")
                }

                input Filter {
                  status: Status @tag(name: \"s\")
                }

                type Query {
                  me(filter: Filter @tag(name: \"f\")): String @auth(requires: ADMIN)
                }
            "}
        );
    }

    #[test]
    fn trims_schema_definition_to_used_roots() {
        let schema = indoc! {"
            schema {
              query: RootQuery
              mutation: RootMutation
              subscription: RootSubscription
            }

            type RootQuery {
              me: String
            }

            type RootMutation {
              rename(name: String): String
            }

            type RootSubscription {
              ticks: Int
            }
        "};

        let query = indoc! {"
            mutation {
              rename(name: \"x\")
            }
        "};

        let result = pruned(schema, query);

        assert_eq!(
            result,
            indoc! {"
                schema {
                  query: RootQuery
                  mutation: RootMutation
                }

                type RootQuery {
                  me: String
                }

                type RootMutation {
                  rename(name: String): String
                }
            "}
        );
    }

    #[test]
    fn keeps_the_query_root_of_a_mutation_only_document() {
        let schema = indoc! {"
            type User {
              id: ID!
            }

            type Query {
              me: User
              search(term: String): [User]
              version: String
            }

            type Mutation {
              login: User
            }
        "};

        let query = indoc! {"
            mutation {
              login {
                id
              }
            }
        "};

        let result = pruned(schema, query);

        assert_eq!(
            result,
            indoc! {"
                type User {
                  id: ID!
                }

                type Query {
                  version: String
                }

                type Mutation {
                  login: User
                }
            "}
        );
    }

    #[test]
    fn keeps_the_query_root_of_a_subscription_only_document() {
        let schema = indoc! {"
            type Query {
              user(id: ID!): User
              status: Status
            }

            enum Status {
              UP
              DOWN
            }

            type User {
              id: ID!
            }

            type Subscription {
              ticks: Int
            }
        "};

        let query = indoc! {"
            subscription {
              ticks
            }
        "};

        let result = pruned(schema, query);

        assert_eq!(
            result,
            indoc! {"
                type Query {
                  status: Status
                }

                enum Status {
                  UP
                  DOWN
                }

                type Subscription {
                  ticks: Int
                }
            "}
        );
    }

    #[test]
    fn keeps_a_query_root_named_by_the_schema_definition() {
        let schema = indoc! {"
            schema {
              query: Root
              mutation: Change
            }

            type Root {
              lookup(key: String): String
              ping: Boolean
            }

            type Query {
              unrelated: String
            }

            type Change {
              reset: Boolean
            }
        "};

        let query = indoc! {"
            mutation {
              reset
            }
        "};

        let result = pruned(schema, query);

        assert_eq!(
            result,
            indoc! {"
                schema {
                  query: Root
                  mutation: Change
                }

                type Root {
                  ping: Boolean
                }

                type Change {
                  reset: Boolean
                }
            "}
        );
    }

    #[test]
    fn keeps_a_query_root_defined_only_by_an_extension() {
        let schema = indoc! {"
            type User {
              id: ID!
            }

            extend type Query {
              me: User
              ping: String
            }

            extend type Query {
              health: Boolean
            }

            type Mutation {
              login: User
            }
        "};

        let query = indoc! {"
            mutation {
              login {
                id
              }
            }
        "};

        // The base `Query` lives in another document, as in a federation
        // subgraph, so the output extends a type it does not define, exactly as
        // the input does. That is why this skips `pruned` and its check for
        // self-containment, and runs the rest of its checks directly.
        let result = prune::process(
            &Input::inline(schema),
            &Input::inline(query),
            &mut Vec::new(),
        )
        .unwrap()
        .document;
        util::assert_no_empty_definitions(&result);
        assert_implementors_complete(&result);
        assert_supports_query(&result, query);
        assert_has_query_root(&result, "Query");

        assert_eq!(
            result,
            indoc! {"
                type User {
                  id: ID!
                }

                extend type Query {
                  ping: String
                }

                type Mutation {
                  login: User
                }
            "}
        );
    }

    #[test]
    fn keeps_the_smallest_form_of_what_the_query_root_field_returns() {
        let schema = indoc! {"
            type Query {
              node(id: ID!): User
              me: User
            }

            type User {
              profile: Profile
              name(format: String): String
              id: ID!
            }

            type Profile {
              bio: String
            }

            type Mutation {
              logout: Boolean
            }
        "};

        let query = indoc! {"
            mutation {
              logout
            }
        "};

        let result = pruned(schema, query);

        assert_eq!(
            result,
            indoc! {"
                type Query {
                  me: User
                }

                type User {
                  id: ID!
                }

                type Mutation {
                  logout: Boolean
                }
            "}
        );
    }

    #[test]
    fn keeps_only_what_a_query_selects_on_the_query_root() {
        let schema = indoc! {"
            type User {
              id: ID!
            }

            type Query {
              me: User
              version: String
            }

            type Mutation {
              login: User
            }
        "};

        let query = indoc! {"
            query {
              me {
                id
              }
            }

            mutation {
              login {
                id
              }
            }
        "};

        let result = pruned(schema, query);

        assert_eq!(
            result,
            indoc! {"
                type User {
                  id: ID!
                }

                type Query {
                  me: User
                }

                type Mutation {
                  login: User
                }
            "}
        );
    }

    #[test]
    fn adds_no_query_root_to_a_schema_without_one() {
        let schema = indoc! {"
            type Mutation {
              login: String
            }

            type Other {
              value: Int
            }
        "};

        let query = indoc! {"
            mutation {
              login
            }
        "};

        let result = pruned(schema, query);

        assert_eq!(
            result,
            indoc! {"
                type Mutation {
                  login: String
                }
            "}
        );
    }

    #[test]
    fn prunes_extension_fields_like_base_fields() {
        let schema = indoc! {"
            type Query {
              me: User
            }

            extend type Query {
              bots: [Bot]
            }

            type User {
              id: ID!
              name: String
            }

            extend type User {
              avatar: Image
              nickname: String
            }

            type Image {
              url: String
            }

            type Bot {
              id: ID!
            }
        "};

        // `bots` is unused, so its extension empties and goes, along with Bot.
        // Only `avatar` survives from the User extension.
        let query = indoc! {"
            {
              me {
                id
                avatar {
                  url
                }
              }
            }
        "};
        let result = pruned(schema, query);
        assert_eq!(
            result,
            indoc! {"
                type Query {
                  me: User
                }

                type User {
                  id: ID!
                }

                extend type User {
                  avatar: Image
                }

                type Image {
                  url: String
                }
            "}
        );

        // A field that exists only in an extension is resolved like any other.
        // The base definition is trimmed to empty, so the extension takes its
        // place as the definition.
        let query = indoc! {"
            {
              bots {
                id
              }
            }
        "};
        let result = pruned(schema, query);
        assert_eq!(
            result,
            indoc! {"
                type Query {
                  bots: [Bot]
                }

                type Bot {
                  id: ID!
                }
            "}
        );
    }

    #[test]
    fn prunes_extension_interfaces_and_union_members_like_base_ones() {
        let schema = indoc! {"
            type Query {
              search: [SearchResult]
              node: Node
            }

            interface Node {
              id: ID!
            }

            type User {
              id: ID!
              name: String
            }

            extend type User implements Node

            type Post {
              title: String
            }

            type Comment {
              body: String
            }

            union SearchResult = User

            extend union SearchResult = Post | Comment
        "};

        // User implements Node only through its extension, so selecting `id`
        // on Node keeps it on User. The union keeps the members the query
        // enters, User (as a Node) and Post, decided across the base and the
        // extension together, so only Comment goes.
        let query = indoc! {"
            {
              search {
                ... on Post {
                  title
                }
              }
              node {
                id
              }
            }
        "};
        let result = pruned(schema, query);
        assert_eq!(
            result,
            indoc! {"
                type Query {
                  search: [SearchResult]
                  node: Node
                }

                interface Node {
                  id: ID!
                }

                type User {
                  id: ID!
                }

                extend type User implements Node

                type Post {
                  title: String
                }

                union SearchResult = User

                extend union SearchResult = Post
            "}
        );
    }

    #[test]
    fn removes_interfaces_trimmed_to_empty_from_implements_clauses() {
        let schema = indoc! {"
            interface Node {
              id: ID!
            }

            interface Named {
              name: String
            }

            type User implements Node & Named {
              id: ID!
              name: String
            }

            type Post {
              id: ID!
              title: String
            }

            extend type Post implements Node

            type Query {
              me: User
              posts: [Post]
            }
        "};

        // Named is entered and keeps `name`. No Node field is selected, so Node
        // goes, User stops implementing it, and the extension that only adds
        // it to Post is left with nothing and dropped.
        let query = indoc! {"
            {
              me {
                ... on Named {
                  name
                }
              }
              posts {
                title
              }
            }
        "};

        let result = pruned(schema, query);

        assert_eq!(
            result,
            indoc! {"
                interface Named {
                  name: String
                }

                type User implements Named {
                  name: String
                }

                type Post {
                  title: String
                }

                type Query {
                  me: User
                  posts: [Post]
                }
            "}
        );
    }

    #[test]
    fn keeps_one_field_of_types_the_query_enters_without_selecting_any() {
        let schema = indoc! {"
            interface Node {
              friends(first: Int): [Node]
              label(format: String): String
              id: ID!
            }

            type User implements Node {
              friends(first: Int): [Node]
              label(format: String): String
              id: ID!
              name: String
            }

            type Viewer {
              user: User
            }

            type Query {
              node: Node
              viewer: Viewer
            }
        "};

        // The query needs Node and Viewer, but selects nothing on them. Node
        // keeps its first field returning a leaf type without arguments, which
        // carries over to its implementor User. Viewer has no such field, so
        // it keeps `user`, whose type User then needs a field, and already has
        // one.
        let query = indoc! {"
            {
              node {
                ... on Node {
                  __typename
                }
              }
              viewer {
                __typename
              }
            }
        "};

        let result = pruned(schema, query);

        assert_eq!(
            result,
            indoc! {"
                interface Node {
                  id: ID!
                }

                type User implements Node {
                  id: ID!
                }

                type Viewer {
                  user: User
                }

                type Query {
                  node: Node
                  viewer: Viewer
                }
            "}
        );
    }

    #[test]
    fn keeps_one_field_of_a_root_type_the_query_selects_only_typename_on() {
        let schema = indoc! {"
            type Query {
              me: User
              version: String
            }

            type User {
              id: ID!
            }
        "};

        let query = indoc! {"
            {
              __typename
            }
        "};

        let result = pruned(schema, query);

        assert_eq!(
            result,
            indoc! {"
                type Query {
                  version: String
                }
            "}
        );
    }

    #[test]
    fn keeps_the_member_an_implementor_narrows_a_union_field_to() {
        let schema = indoc! {"
            type Query {
              holder: HasResult
            }

            interface HasResult {
              result: SearchResult
            }

            union SearchResult = Post | User

            type Post {
              title: String
            }

            type User {
              id: ID!
            }

            type Holder implements HasResult {
              result: User
            }

            type Box implements HasResult {
              result: SearchResult
            }
        "};

        // Holder must keep `result`, which it narrows to User, so User is
        // entered and stays a member of SearchResult, and it keeps its
        // smallest valid form, which then satisfies SearchResult too.
        let query = indoc! {"
            {
              holder {
                result {
                  __typename
                }
              }
            }
        "};
        let result = pruned(schema, query);
        assert_eq!(
            result,
            indoc! {"
                type Query {
                  holder: HasResult
                }

                interface HasResult {
                  result: SearchResult
                }

                union SearchResult = User

                type User {
                  id: ID!
                }

                type Holder implements HasResult {
                  result: User
                }

                type Box implements HasResult {
                  result: SearchResult
                }
            "}
        );

        // The same holds when the query enters only another member.
        let query = indoc! {"
            {
              holder {
                result {
                  ... on Post {
                    title
                  }
                }
              }
            }
        "};
        let result = pruned(schema, query);
        assert_eq!(
            result,
            indoc! {"
                type Query {
                  holder: HasResult
                }

                interface HasResult {
                  result: SearchResult
                }

                union SearchResult = Post | User

                type Post {
                  title: String
                }

                type User {
                  id: ID!
                }

                type Holder implements HasResult {
                  result: User
                }

                type Box implements HasResult {
                  result: SearchResult
                }
            "}
        );
    }

    #[test]
    fn keeps_the_member_an_implementor_narrows_a_wrapped_union_field_to() {
        let schema = indoc! {"
            type Query {
              holder: HasResults
            }

            interface HasResults {
              results: [SearchResult]
            }

            union SearchResult = Post | User

            type Post {
              title: String
            }

            type User {
              id: ID!
            }

            type Holder implements HasResults {
              results: [User!]!
            }
        "};

        let query = indoc! {"
            {
              holder {
                results {
                  ... on Post {
                    title
                  }
                }
              }
            }
        "};

        let result = pruned(schema, query);

        assert_eq!(
            result,
            indoc! {"
                type Query {
                  holder: HasResults
                }

                interface HasResults {
                  results: [SearchResult]
                }

                union SearchResult = Post | User

                type Post {
                  title: String
                }

                type User {
                  id: ID!
                }

                type Holder implements HasResults {
                  results: [User!]!
                }
            "}
        );
    }

    #[test]
    fn keeps_what_an_object_narrowed_from_an_interface_field_needs() {
        let schema = indoc! {"
            type Query {
              owner: HasNode
            }

            interface HasNode {
              node: Node
            }

            type Holder implements HasNode {
              node: Post
            }

            interface Node {
              id: ID!
              attachment: Attachment
            }

            type Post implements Node {
              id: ID!
              attachment: Image
            }

            union Attachment = Image | Video

            type Image {
              url: String
            }

            type Video {
              length: Int
            }
        "};

        // Holder narrows `node` to Post, which is kept as a Node, and Post in
        // turn narrows `attachment` to Image, which the query never enters.
        // Image is kept all the same, so Post still satisfies Node.
        let query = indoc! {"
            {
              owner {
                node {
                  attachment {
                    ... on Video {
                      length
                    }
                  }
                }
              }
            }
        "};

        let result = pruned(schema, query);

        assert_eq!(
            result,
            indoc! {"
                type Query {
                  owner: HasNode
                }

                interface HasNode {
                  node: Node
                }

                type Holder implements HasNode {
                  node: Post
                }

                interface Node {
                  attachment: Attachment
                }

                type Post implements Node {
                  attachment: Image
                }

                union Attachment = Image | Video

                type Image {
                  url: String
                }

                type Video {
                  length: Int
                }
            "}
        );
    }

    #[test]
    fn keeps_what_each_interface_in_a_chain_narrows_its_fields_to() {
        let schema = indoc! {"
            type Query {
              a: A
            }

            interface A {
              owner: Node
              result: SearchResult
            }

            interface B implements A {
              owner: Named
              result: User
            }

            type C implements B & A {
              owner: Person
              result: User
            }

            interface Node {
              id: ID!
            }

            interface Named implements Node {
              id: ID!
              name: String
            }

            type Person implements Named & Node {
              id: ID!
              name: String
            }

            union SearchResult = Post | User

            type Post {
              title: String
            }

            type User {
              id: ID!
            }
        "};

        // The query only goes through A, and never enters B, Named, or User.
        // B is kept because C implements it, and keeps A's fields, so what it
        // narrows them to is kept too.
        let query = indoc! {"
            {
              a {
                owner {
                  id
                }
                result {
                  ... on Post {
                    title
                  }
                }
              }
            }
        "};

        let result = pruned(schema, query);

        assert_eq!(
            result,
            indoc! {"
                type Query {
                  a: A
                }

                interface A {
                  owner: Node
                  result: SearchResult
                }

                interface B implements A {
                  owner: Named
                  result: User
                }

                type C implements B & A {
                  owner: Person
                  result: User
                }

                interface Node {
                  id: ID!
                }

                interface Named implements Node {
                  id: ID!
                }

                type Person implements Named & Node {
                  id: ID!
                }

                union SearchResult = Post | User

                type Post {
                  title: String
                }

                type User {
                  id: ID!
                }
            "}
        );
    }

    #[test]
    fn promotes_the_first_surviving_extension_over_a_base_trimmed_to_empty() {
        let schema = indoc! {"
            \"The root\"
            type Query @tag {
              me: User
            }

            extend type Query {
              bots: [Bot]
            }

            extend type Query {
              admins: [Bot]
            }

            directive @tag on OBJECT

            directive @key(fields: String) on OBJECT

            \"A user\"
            type User @key(fields: \"id\") {
              id: ID!
            }

            extend type User implements Node

            extend type User {
              name: String
            }

            extend type User {
              email: String
            }

            interface Node {
              name: String
            }

            type Bot {
              id: ID!
            }
        "};

        // The first surviving extension becomes the definition, carrying the
        // base's description and directives; later ones stay extensions.
        let query = indoc! {"
            {
              bots {
                id
              }
              admins {
                id
              }
            }
        "};
        let result = pruned(schema, query);
        assert_eq!(
            result,
            indoc! {"
                \"The root\"
                type Query @tag {
                  bots: [Bot]
                }

                extend type Query {
                  admins: [Bot]
                }

                directive @tag on OBJECT

                type Bot {
                  id: ID!
                }
            "}
        );

        // The first surviving User extension only adds Node, so the definition
        // it becomes also takes in the next one, to have a field.
        let query = indoc! {"
            {
              me {
                ... on Node {
                  name
                }
                email
              }
            }
        "};
        let result = pruned(schema, query);
        assert_eq!(
            result,
            indoc! {"
                \"The root\"
                type Query @tag {
                  me: User
                }

                directive @tag on OBJECT

                directive @key(fields: String) on OBJECT

                \"A user\"
                type User implements Node @key(fields: \"id\") {
                  name: String
                }

                extend type User {
                  email: String
                }

                interface Node {
                  name: String
                }
            "}
        );
    }

    const SUPERGRAPH: &str = include_str!("../tests/fixtures/supergraph.graphql");

    /// The types of a pruned supergraph that are not its machinery, in order.
    fn supergraph_types(schema: &str) -> String {
        let doc = parse_schema::<String>(schema).unwrap();
        let sg = Supergraph::detect(&doc).unwrap();
        doc.definitions
            .iter()
            .filter_map(|def| match def {
                SchemaDef::TypeDefinition(td)
                    if !sg.is_machinery_type(util::type_definition_name(td)) =>
                {
                    Some(td.to_string())
                }
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// The fields a key, `@requires`, or `@provides` names are kept, since the
    /// router fetches them to resolve what the query selects, though the query
    /// never does.
    #[test]
    fn keeps_the_fields_keys_requires_and_provides_name() {
        let query = include_str!("../tests/fixtures/supergraph-query.graphql");
        assert_eq!(
            supergraph_types(&pruned(SUPERGRAPH, query)),
            indoc! {r#"
                type Product @join__type(graph: PRODUCTS, key: "upc") @join__type(graph: REVIEWS, key: "upc") {
                  upc: String!
                  name: String @join__field(graph: PRODUCTS, overrideLabel: "percent(50)") @join__field(graph: REVIEWS, override: "products", overrideLabel: "percent(50)")
                  weight: Int @join__field(graph: PRODUCTS) @join__field(graph: REVIEWS, external: true)
                  shipping: Int @join__field(graph: REVIEWS, requires: "weight")
                  reviews: [Review] @join__field(graph: REVIEWS)
                }

                type Query @join__type(graph: ACCOUNTS) @join__type(graph: PRODUCTS) @join__type(graph: REVIEWS) {
                  topProducts(first: Int = 5): [Product] @join__field(graph: PRODUCTS)
                }

                type Review @join__type(graph: REVIEWS) {
                  body: String
                  author: User @join__field(graph: REVIEWS, provides: "username")
                }

                type User @join__type(graph: ACCOUNTS, key: "id") @join__type(graph: PRODUCTS, key: "id", resolvable: false) @join__type(graph: REVIEWS, key: "id") {
                  id: ID!
                  username: String @join__field(graph: ACCOUNTS) @join__field(graph: REVIEWS, external: true)
                }
            "#}
        );
    }

    /// A key's nested selection enters the types it passes through.
    #[test]
    fn keeps_nested_key_fields() {
        let types = supergraph_types(&pruned(SUPERGRAPH, "{ teams { rating } }"));
        assert!(
            types.contains(indoc! {r#"
                type Org @join__type(graph: ACCOUNTS, key: "id") @join__type(graph: REVIEWS, key: "id", resolvable: false) {
                  id: ID!
                }
            "#}),
            "{types}"
        );
        assert!(types.contains("  slug: String!\n  org: Org!\n"), "{types}");
    }

    /// The join directives follow what prune removes, so none says a graph
    /// has an interface the output lacks.
    #[test]
    fn drops_the_join_directives_of_what_it_removes() {
        let types = supergraph_types(&pruned(SUPERGRAPH, "{ topProducts { upc } }"));
        assert!(
            types.contains(indoc! {r#"
                type Product @join__type(graph: PRODUCTS, key: "upc") @join__type(graph: REVIEWS, key: "upc") {
                  upc: String!
                }
            "#}),
            "{types}"
        );
    }

    /// Every subgraph resolving a kept field keeps a field of the type it
    /// returns there, or its extraction would lose the type and the field
    /// with it. Here only `B` resolves `Query.b`, and `Pair` has only `y` in
    /// `B`, which the query never selects.
    #[test]
    fn keeps_a_field_of_what_a_kept_field_returns_in_each_graph_resolving_it() {
        let schema = indoc! {r#"
            schema @link(url: "https://specs.apollo.dev/link/v1.0") @link(url: "https://specs.apollo.dev/join/v0.4", for: EXECUTION) { query: Query }
            directive @join__type(graph: join__Graph!, key: join__FieldSet) repeatable on OBJECT
            directive @join__field(graph: join__Graph) repeatable on FIELD_DEFINITION
            directive @join__graph(name: String!, url: String!) on ENUM_VALUE
            directive @link(url: String, as: String, for: link__Purpose, import: [link__Import]) repeatable on SCHEMA
            scalar join__FieldSet
            scalar link__Import
            enum link__Purpose { SECURITY EXECUTION }
            enum join__Graph {
              A @join__graph(name: "a", url: "http://a")
              B @join__graph(name: "b", url: "http://b")
            }
            type Query @join__type(graph: A) @join__type(graph: B) {
              a: Pair @join__field(graph: A)
              b: Pair @join__field(graph: B)
            }
            type Pair @join__type(graph: A) @join__type(graph: B) {
              x: Int @join__field(graph: A)
              y: Int @join__field(graph: B)
            }
        "#};
        assert_eq!(
            supergraph_types(&pruned(schema, "{ a { x } b { __typename } }")),
            indoc! {"
                type Query @join__type(graph: A) @join__type(graph: B) {
                  a: Pair @join__field(graph: A)
                  b: Pair @join__field(graph: B)
                }

                type Pair @join__type(graph: A) @join__type(graph: B) {
                  x: Int @join__field(graph: A)
                  y: Int @join__field(graph: B)
                }
            "}
        );
    }
}
