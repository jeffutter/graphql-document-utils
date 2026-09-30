use graphql_parser::query::{
    Directive, Mutation, OperationDefinition, Query, SelectionSet, Subscription, Text,
    TypeCondition, Value, VariableDefinition,
};
use graphql_parser::schema::{
    Definition, DirectiveDefinition, Document, EnumType, EnumTypeExtension, Field, InputObjectType,
    InputObjectTypeExtension, InputValue, InterfaceType, InterfaceTypeExtension, ObjectType,
    ObjectTypeExtension, ScalarType, ScalarTypeExtension, SchemaDefinition, Type, TypeDefinition,
    TypeExtension, UnionType, UnionTypeExtension,
};
use std::collections::{HashMap, HashSet};

/// The scalars every schema has without defining them.
pub const BUILT_IN_SCALARS: [&str; 5] = ["Boolean", "Float", "ID", "Int", "String"];

pub fn schema_type_definition_name<'a, V, D: Text<'a, Value = V>>(
    td: &'a TypeDefinition<'a, D>,
) -> Option<&'a V> {
    match td {
        TypeDefinition::Scalar(scalar_type) => Some(&scalar_type.name),
        TypeDefinition::Object(object_type) => Some(&object_type.name),
        TypeDefinition::Interface(interface_type) => Some(&interface_type.name),
        TypeDefinition::Union(union_type) => Some(&union_type.name),
        TypeDefinition::Enum(enum_type) => Some(&enum_type.name),
        TypeDefinition::InputObject(input_object_type) => Some(&input_object_type.name),
    }
}

pub fn named_type<'r, 'a, D: Text<'a>>(ty: &'r Type<'a, D>) -> Option<&'r D::Value> {
    match ty {
        Type::NamedType(n) => Some(n),
        Type::ListType(inner) | Type::NonNullType(inner) => named_type(inner),
    }
}

/// Retrieves the fields of an object or interface type. Other type definitions
/// have no selectable fields.
pub fn type_fields<'r, 'a, D: Text<'a>>(
    td: &'r TypeDefinition<'a, D>,
) -> Option<&'r Vec<Field<'a, D>>> {
    match td {
        TypeDefinition::Object(obj) => Some(&obj.fields),
        TypeDefinition::Interface(iface) => Some(&iface.fields),
        _ => None,
    }
}

/// The type names backing each operation root, as named by the schema
/// definition. `query` defaults to `Query` when the schema leaves it implicit;
/// `mutation` and `subscription` are absent unless the schema declares them.
pub struct RootTypes {
    pub query: String,
    pub mutation: Option<String>,
    pub subscription: Option<String>,
}

/// Detects root types (Query, Mutation, Subscription) from the schema.
pub fn detect_root_types(schema: &Document<String>) -> RootTypes {
    let mut root = RootTypes {
        query: "Query".to_string(),
        mutation: None,
        subscription: None,
    };

    for def in &schema.definitions {
        if let Definition::SchemaDefinition(schema_def) = def {
            if let Some(query) = &schema_def.query {
                root.query = query.clone();
            }
            if let Some(mutation) = &schema_def.mutation {
                root.mutation = Some(mutation.clone());
            }
            if let Some(subscription) = &schema_def.subscription {
                root.subscription = Some(subscription.clone());
            }
        }
    }

    root
}

/// The root type name and selection set an operation starts from. Anonymous
/// operations and bare selection sets root at the query type.
pub fn operation_root<'a, 'q>(
    root_types: &'a RootTypes,
    op: &'a OperationDefinition<'q, String>,
) -> (&'a str, &'a SelectionSet<'q, String>) {
    match op {
        OperationDefinition::SelectionSet(set) => (&root_types.query, set),
        OperationDefinition::Query(query) => (&root_types.query, &query.selection_set),
        OperationDefinition::Mutation(mutation) => (
            root_types.mutation.as_deref().unwrap_or("Mutation"),
            &mutation.selection_set,
        ),
        OperationDefinition::Subscription(subscription) => (
            root_types.subscription.as_deref().unwrap_or("Subscription"),
            &subscription.selection_set,
        ),
    }
}

pub fn operation_directives<'a, 'q>(
    op: &'a OperationDefinition<'q, String>,
) -> &'a [Directive<'q, String>] {
    match op {
        OperationDefinition::SelectionSet(_) => &[],
        OperationDefinition::Query(query) => &query.directives,
        OperationDefinition::Mutation(mutation) => &mutation.directives,
        OperationDefinition::Subscription(subscription) => &subscription.directives,
    }
}

/// Rebuilds an operation around a rewritten selection set, directives, and
/// variable definitions. A bare selection set has nowhere to put directives or
/// variables, so both are ignored for that form.
pub fn rebuild_operation<'q>(
    op: &OperationDefinition<'q, String>,
    selection_set: SelectionSet<'q, String>,
    directives: Vec<Directive<'q, String>>,
    variable_definitions: Vec<VariableDefinition<'q, String>>,
) -> OperationDefinition<'q, String> {
    match op {
        OperationDefinition::SelectionSet(_) => OperationDefinition::SelectionSet(selection_set),
        OperationDefinition::Query(query) => OperationDefinition::Query(Query {
            variable_definitions,
            directives,
            selection_set,
            ..query.clone()
        }),
        OperationDefinition::Mutation(mutation) => OperationDefinition::Mutation(Mutation {
            variable_definitions,
            directives,
            selection_set,
            ..mutation.clone()
        }),
        OperationDefinition::Subscription(subscription) => {
            OperationDefinition::Subscription(Subscription {
                variable_definitions,
                directives,
                selection_set,
                ..subscription.clone()
            })
        }
    }
}

/// The variable definitions an operation declares, in source order.
pub fn operation_variable_definitions<'a, 'q>(
    op: &'a OperationDefinition<'q, String>,
) -> &'a [VariableDefinition<'q, String>] {
    match op {
        OperationDefinition::SelectionSet(_) => &[],
        OperationDefinition::Query(query) => &query.variable_definitions,
        OperationDefinition::Mutation(mutation) => &mutation.variable_definitions,
        OperationDefinition::Subscription(subscription) => &subscription.variable_definitions,
    }
}

pub fn type_condition<'a>(condition: Option<&'a TypeCondition<'_, String>>) -> Option<&'a str> {
    condition.map(|TypeCondition::On(name)| name.as_str())
}

pub fn collect_directive_variables(
    directives: &[Directive<'_, String>],
    used: &mut HashSet<String>,
) {
    for directive in directives {
        for (_name, value) in &directive.arguments {
            collect_value_variables(value, used);
        }
    }
}

pub fn collect_value_variables(value: &Value<'_, String>, used: &mut HashSet<String>) {
    match value {
        Value::Variable(name) => {
            used.insert(name.clone());
        }
        Value::List(values) => {
            for value in values {
                collect_value_variables(value, used);
            }
        }
        Value::Object(fields) => {
            for value in fields.values() {
                collect_value_variables(value, used);
            }
        }
        Value::Int(_)
        | Value::Float(_)
        | Value::String(_)
        | Value::Boolean(_)
        | Value::Null
        | Value::Enum(_) => (),
    }
}

/// Every named type `schema` defines, keyed by name, with each `extend` block
/// folded into the type it extends. Extension fields, implemented interfaces,
/// union members, enum values, input fields, and directives are appended in
/// source order, so a command asking what a type has sees all of it, wherever
/// in the document it was written.
///
/// An extension whose base type the document does not define stands in as the
/// definition, and later extensions fold into it. That is the usual shape of a
/// schema split across documents, such as a federation subgraph that only
/// writes `extend type Query`, and the extension is then the only record of
/// what the type has. Dropping it instead would make every field it adds look
/// undefined. An extension of a different kind than the type it names (an
/// `extend union` of an object type) is invalid and cannot be folded, so it is
/// ignored.
///
/// A name defined more than once keeps its first definition, though commands
/// never pass one: `Input::parse_schema` has already dropped the rest, with a
/// warning.
pub fn merged_type_definitions<'a>(
    schema: &Document<'a, String>,
) -> HashMap<String, TypeDefinition<'a, String>> {
    let mut merged = HashMap::new();
    for def in &schema.definitions {
        if let Definition::TypeDefinition(td) = def {
            merged
                .entry(type_definition_name(td).clone())
                .or_insert_with(|| td.clone());
        }
    }

    // Extensions fold in after every base definition is indexed, since an
    // extension may appear before the type it extends.
    for def in &schema.definitions {
        let Definition::TypeExtension(te) = def else {
            continue;
        };
        let extension = extension_as_definition(te);
        match merged.get_mut(type_extension_name(te)) {
            Some(base) => fold_extension(base, extension),
            None => {
                merged.insert(type_extension_name(te).clone(), extension);
            }
        }
    }

    merged
}

/// Appends what `extension` (read as a definition) adds to `base`: its
/// implemented interfaces, directives, and members, after `base`'s own. An
/// extension of a different kind than `base` is invalid and is ignored.
fn fold_extension<'a>(
    base: &mut TypeDefinition<'a, String>,
    extension: TypeDefinition<'a, String>,
) {
    match (base, extension) {
        (TypeDefinition::Scalar(base), TypeDefinition::Scalar(ext)) => {
            base.directives.extend(ext.directives);
        }
        (TypeDefinition::Object(base), TypeDefinition::Object(ext)) => {
            base.implements_interfaces.extend(ext.implements_interfaces);
            base.directives.extend(ext.directives);
            base.fields.extend(ext.fields);
        }
        (TypeDefinition::Interface(base), TypeDefinition::Interface(ext)) => {
            base.implements_interfaces.extend(ext.implements_interfaces);
            base.directives.extend(ext.directives);
            base.fields.extend(ext.fields);
        }
        (TypeDefinition::Union(base), TypeDefinition::Union(ext)) => {
            base.directives.extend(ext.directives);
            base.types.extend(ext.types);
        }
        (TypeDefinition::Enum(base), TypeDefinition::Enum(ext)) => {
            base.directives.extend(ext.directives);
            base.values.extend(ext.values);
        }
        (TypeDefinition::InputObject(base), TypeDefinition::InputObject(ext)) => {
            base.directives.extend(ext.directives);
            base.fields.extend(ext.fields);
        }
        _ => (),
    }
}

/// Whether `td` is an object or interface with no fields, or a union with no
/// members, none of which is valid SDL. Other kinds are never empty here,
/// since nothing trims their members.
fn has_no_members(td: &TypeDefinition<'_, String>) -> bool {
    match td {
        TypeDefinition::Object(obj) => obj.fields.is_empty(),
        TypeDefinition::Interface(iface) => iface.fields.is_empty(),
        TypeDefinition::Union(union) => union.types.is_empty(),
        TypeDefinition::Scalar(_) | TypeDefinition::Enum(_) | TypeDefinition::InputObject(_) => {
            false
        }
    }
}

/// Returns the part of `schema` that the `types` and `directives` named need
/// in order to stand on their own, so no retained definition names one that
/// was dropped.
///
/// Every named type the schema defines is kept after passing through `trim`,
/// and so is everything the trimmed definition names: field return types,
/// argument types, input object field types (recursively), implemented
/// interfaces, union members, and the definitions of every directive applied
/// to the type, its fields, arguments, enum values, or input fields, along with
/// those directives' own argument types. Names the schema does not define,
/// such as built-in scalars and directives, are skipped.
///
/// `trim` decides how much of each type survives, and references are followed
/// from what it returns, so it is the one place a caller narrows the result.
/// It sees each type whole, with its extensions merged in (see
/// `merged_type_definitions`), and should only remove members, not rewrite or
/// add them, and should leave enums and input objects whole. Return the
/// definition unchanged to keep types whole.
///
/// An object or interface that trimming leaves with no fields, or a union it
/// leaves with no members, is removed, since an empty one is not valid SDL.
/// So is every reference to it: fields returning it, union members naming it,
/// and `implements` clauses naming it, which can empty further types in turn.
/// A type that was already empty in the source is not "left" empty by
/// trimming, and is kept as written. Types named in `types` are removed like
/// any other, so a caller that needs one keeps some of it in `trim`.
///
/// Only dependencies are followed, never possible types: a retained interface
/// does not bring its implementors, since nothing in the interface names them.
/// Callers that want implementors kept include them in `types`.
///
/// The output keeps the source layout. A retained type's base definition and
/// each of its extensions keep whichever of their own members survived
/// trimming, and an extension left with nothing is dropped, since an empty
/// `extend` block is not valid. A base definition that trimming leaves without
/// fields or members gives way to its first surviving extension, which is
/// emitted as the definition, carrying the base's description, interfaces,
/// and directives, so no `extend` is left without a type to extend. If that
/// extension adds no fields or members itself, it takes in the ones after it
/// until it does. The schema definition is kept with only the root operation
/// types that were retained, and dropped when none were. Everything keeps its
/// source order.
pub fn retain_with_dependencies<'a>(
    schema: &Document<'a, String>,
    types: &[&str],
    directives: &[&str],
    trim: impl Fn(&TypeDefinition<'a, String>) -> TypeDefinition<'a, String>,
) -> Document<'a, String> {
    let type_index = merged_type_definitions(schema);

    // Every type is trimmed up front, not just the ones reached, because
    // whether a type survives depends on whether the types it names do, and a
    // removal can empty the types naming it. Repeat until nothing more empties.
    // What is left is what each type keeps if retained.
    let mut trimmed: HashMap<String, TypeDefinition<'a, String>> = type_index
        .iter()
        .map(|(name, td)| (name.clone(), trim(td)))
        .collect();
    let mut removed: HashSet<String> = HashSet::new();
    loop {
        let emptied: Vec<String> = trimmed
            .iter()
            .filter(|(name, td)| has_no_members(td) && type_index.get(*name) != Some(*td))
            .map(|(name, _)| name.clone())
            .collect();
        if emptied.is_empty() {
            break;
        }
        for name in emptied {
            trimmed.remove(&name);
            removed.insert(name);
        }
        let names_removed =
            |ty: &Type<'_, String>| named_type(ty).is_some_and(|n| removed.contains(n));
        for td in trimmed.values_mut() {
            match td {
                TypeDefinition::Object(obj) => {
                    obj.implements_interfaces
                        .retain(|name| !removed.contains(name));
                    obj.fields.retain(|field| !names_removed(&field.field_type));
                }
                TypeDefinition::Interface(iface) => {
                    iface
                        .implements_interfaces
                        .retain(|name| !removed.contains(name));
                    iface
                        .fields
                        .retain(|field| !names_removed(&field.field_type));
                }
                TypeDefinition::Union(union) => union.types.retain(|name| !removed.contains(name)),
                TypeDefinition::Scalar(_)
                | TypeDefinition::Enum(_)
                | TypeDefinition::InputObject(_) => (),
            }
        }
    }

    let mut directive_index = HashMap::new();
    let mut schema_definition = None;
    for def in &schema.definitions {
        match def {
            Definition::SchemaDefinition(sd) => schema_definition = Some(sd),
            Definition::DirectiveDefinition(dd) => {
                directive_index.insert(dd.name.as_str(), dd);
            }
            Definition::TypeDefinition(_) | Definition::TypeExtension(_) => (),
        }
    }

    let mut kept_types: HashMap<String, TypeDefinition<'a, String>> = HashMap::new();
    let mut kept_directives: HashSet<String> = HashSet::new();
    let mut pending = References {
        types: types.iter().map(|name| name.to_string()).collect(),
        directives: directives.iter().map(|name| name.to_string()).collect(),
    };
    let mut schema_definition_kept = false;

    // A retained type can apply a directive whose arguments name further types,
    // and a retained root brings the schema definition's own directives, so
    // drain both worklists until neither grows.
    loop {
        while let Some(name) = pending.types.pop() {
            // Undefined, removed, and already kept types are all absent.
            let Some(td) = trimmed.remove(&name) else {
                continue;
            };
            pending.type_definition(&td);
            kept_types.insert(name, td);
        }

        while let Some(name) = pending.directives.pop() {
            if kept_directives.contains(&name) {
                continue;
            }
            if let Some(dd) = directive_index.get(name.as_str()) {
                pending.directive_definition(dd);
                kept_directives.insert(name);
            }
        }

        if let Some(sd) = schema_definition.filter(|_| !schema_definition_kept) {
            if [&sd.query, &sd.mutation, &sd.subscription]
                .into_iter()
                .any(|root| {
                    root.as_ref()
                        .is_some_and(|name| kept_types.contains_key(name))
                })
            {
                schema_definition_kept = true;
                pending.directives(&sd.directives);
            }
        }

        if pending.types.is_empty() && pending.directives.is_empty() {
            break;
        }
    }

    let keep_root =
        |root: &Option<String>| root.clone().filter(|name| kept_types.contains_key(name));
    let schema_definition = schema_definition
        .filter(|_| schema_definition_kept)
        .map(|sd| SchemaDefinition {
            query: keep_root(&sd.query),
            mutation: keep_root(&sd.mutation),
            subscription: keep_root(&sd.subscription),
            ..sd.clone()
        });

    // What `piece` (a base definition or an extension, read as a definition)
    // still has once its type is trimmed to `kept`. Members are matched by name
    // and taken as `kept` has them. `None` when the piece is of a different
    // kind than the type, which only an invalid schema has.
    let surviving_part = |piece: &TypeDefinition<'a, String>| {
        let kept = kept_types.get(type_definition_name(piece))?;
        let named = |own: &[String], kept: &[String]| surviving(own, kept, PartialEq::eq);
        let directives = |own: &[Directive<'a, String>], kept: &[Directive<'a, String>]| {
            surviving(own, kept, PartialEq::eq)
        };
        let fields = |own: &[Field<'a, String>], kept: &[Field<'a, String>]| {
            surviving(own, kept, |a, b| a.name == b.name)
        };
        let input_values = |own: &[InputValue<'a, String>], kept: &[InputValue<'a, String>]| {
            surviving(own, kept, |a, b| a.name == b.name)
        };
        Some(match (piece, kept) {
            (TypeDefinition::Scalar(own), TypeDefinition::Scalar(kept)) => {
                TypeDefinition::Scalar(ScalarType {
                    directives: directives(&own.directives, &kept.directives),
                    ..own.clone()
                })
            }
            (TypeDefinition::Object(own), TypeDefinition::Object(kept)) => {
                TypeDefinition::Object(ObjectType {
                    implements_interfaces: named(
                        &own.implements_interfaces,
                        &kept.implements_interfaces,
                    ),
                    directives: directives(&own.directives, &kept.directives),
                    fields: fields(&own.fields, &kept.fields),
                    ..own.clone()
                })
            }
            (TypeDefinition::Interface(own), TypeDefinition::Interface(kept)) => {
                TypeDefinition::Interface(InterfaceType {
                    implements_interfaces: named(
                        &own.implements_interfaces,
                        &kept.implements_interfaces,
                    ),
                    directives: directives(&own.directives, &kept.directives),
                    fields: fields(&own.fields, &kept.fields),
                    ..own.clone()
                })
            }
            (TypeDefinition::Union(own), TypeDefinition::Union(kept)) => {
                TypeDefinition::Union(UnionType {
                    directives: directives(&own.directives, &kept.directives),
                    types: named(&own.types, &kept.types),
                    ..own.clone()
                })
            }
            (TypeDefinition::Enum(own), TypeDefinition::Enum(kept)) => {
                TypeDefinition::Enum(EnumType {
                    directives: directives(&own.directives, &kept.directives),
                    values: surviving(&own.values, &kept.values, |a, b| a.name == b.name),
                    ..own.clone()
                })
            }
            (TypeDefinition::InputObject(own), TypeDefinition::InputObject(kept)) => {
                TypeDefinition::InputObject(InputObjectType {
                    directives: directives(&own.directives, &kept.directives),
                    fields: input_values(&own.fields, &kept.fields),
                    ..own.clone()
                })
            }
            _ => return None,
        })
    };

    // What `te` still extends its type with, or `None` when nothing survives.
    let surviving_extension = |te: &TypeExtension<'a, String>| {
        surviving_part(&extension_as_definition(te)).and_then(definition_as_extension)
    };

    // A base definition trimmed to no members gives way to its first surviving
    // extension, which is emitted in its place as the definition. That one
    // absorbs the extensions after it until it has members, in case it only
    // adds interfaces or directives. The type survived, so some extension has
    // members. Maps each affected position to what is emitted there instead.
    let mut replacements: HashMap<usize, Option<Definition<'a, String>>> = HashMap::new();
    for (base_index, def) in schema.definitions.iter().enumerate() {
        let Definition::TypeDefinition(td) = def else {
            continue;
        };
        let Some(mut promoted) =
            surviving_part(td).filter(|part| has_no_members(part) && part != td)
        else {
            continue;
        };
        let mut absorbed = Vec::new();
        for (index, def) in schema.definitions.iter().enumerate() {
            let Definition::TypeExtension(te) = def else {
                continue;
            };
            if type_extension_name(te) != type_definition_name(td) {
                continue;
            }
            let Some(extension) = surviving_extension(te) else {
                continue;
            };
            fold_extension(&mut promoted, extension_as_definition(&extension));
            absorbed.push(index);
            if !has_no_members(&promoted) {
                break;
            }
        }
        if let Some((&first, rest)) = absorbed.split_first() {
            replacements.insert(base_index, None);
            replacements.insert(first, Some(Definition::TypeDefinition(promoted)));
            replacements.extend(rest.iter().map(|&index| (index, None)));
        }
    }

    let definitions = schema
        .definitions
        .iter()
        .enumerate()
        .filter_map(|(index, def)| match replacements.remove(&index) {
            Some(replacement) => replacement,
            None => match def {
                Definition::SchemaDefinition(_) => {
                    schema_definition.clone().map(Definition::SchemaDefinition)
                }
                Definition::TypeDefinition(td) => {
                    surviving_part(td).map(Definition::TypeDefinition)
                }
                Definition::TypeExtension(te) => {
                    surviving_extension(te).map(Definition::TypeExtension)
                }
                Definition::DirectiveDefinition(dd) => {
                    kept_directives.contains(&dd.name).then(|| def.clone())
                }
            },
        })
        .collect();

    Document { definitions }
}

/// The items of `own` that are still in `kept`, in `own`'s order, as `kept`
/// has them.
fn surviving<T: Clone>(own: &[T], kept: &[T], same: impl Fn(&T, &T) -> bool) -> Vec<T> {
    own.iter()
        .filter_map(|item| kept.iter().find(|k| same(item, k)))
        .cloned()
        .collect()
}

/// Reads an extension as the definition it would be on its own. The two differ
/// only in that an extension has no description, so treating extensions as
/// definitions lets merging, trimming, and reference walking handle both alike.
fn extension_as_definition<'a>(te: &TypeExtension<'a, String>) -> TypeDefinition<'a, String> {
    match te.clone() {
        TypeExtension::Scalar(ext) => TypeDefinition::Scalar(ScalarType {
            position: ext.position,
            description: None,
            name: ext.name,
            directives: ext.directives,
        }),
        TypeExtension::Object(ext) => TypeDefinition::Object(ObjectType {
            position: ext.position,
            description: None,
            name: ext.name,
            implements_interfaces: ext.implements_interfaces,
            directives: ext.directives,
            fields: ext.fields,
        }),
        TypeExtension::Interface(ext) => TypeDefinition::Interface(InterfaceType {
            position: ext.position,
            description: None,
            name: ext.name,
            implements_interfaces: ext.implements_interfaces,
            directives: ext.directives,
            fields: ext.fields,
        }),
        TypeExtension::Union(ext) => TypeDefinition::Union(UnionType {
            position: ext.position,
            description: None,
            name: ext.name,
            directives: ext.directives,
            types: ext.types,
        }),
        TypeExtension::Enum(ext) => TypeDefinition::Enum(EnumType {
            position: ext.position,
            description: None,
            name: ext.name,
            directives: ext.directives,
            values: ext.values,
        }),
        TypeExtension::InputObject(ext) => TypeDefinition::InputObject(InputObjectType {
            position: ext.position,
            description: None,
            name: ext.name,
            directives: ext.directives,
            fields: ext.fields,
        }),
    }
}

/// The inverse of `extension_as_definition`, or `None` when the definition has
/// nothing to extend with, since an empty `extend` block does not parse.
fn definition_as_extension<'a>(
    td: TypeDefinition<'a, String>,
) -> Option<TypeExtension<'a, String>> {
    let extension = match td {
        TypeDefinition::Scalar(def) => TypeExtension::Scalar(ScalarTypeExtension {
            position: def.position,
            name: def.name,
            directives: def.directives,
        }),
        TypeDefinition::Object(def) => TypeExtension::Object(ObjectTypeExtension {
            position: def.position,
            name: def.name,
            implements_interfaces: def.implements_interfaces,
            directives: def.directives,
            fields: def.fields,
        }),
        TypeDefinition::Interface(def) => TypeExtension::Interface(InterfaceTypeExtension {
            position: def.position,
            name: def.name,
            implements_interfaces: def.implements_interfaces,
            directives: def.directives,
            fields: def.fields,
        }),
        TypeDefinition::Union(def) => TypeExtension::Union(UnionTypeExtension {
            position: def.position,
            name: def.name,
            directives: def.directives,
            types: def.types,
        }),
        TypeDefinition::Enum(def) => TypeExtension::Enum(EnumTypeExtension {
            position: def.position,
            name: def.name,
            directives: def.directives,
            values: def.values,
        }),
        TypeDefinition::InputObject(def) => TypeExtension::InputObject(InputObjectTypeExtension {
            position: def.position,
            name: def.name,
            directives: def.directives,
            fields: def.fields,
        }),
    };
    let empty = match &extension {
        TypeExtension::Scalar(ext) => ext.directives.is_empty(),
        TypeExtension::Object(ext) => {
            ext.implements_interfaces.is_empty()
                && ext.directives.is_empty()
                && ext.fields.is_empty()
        }
        TypeExtension::Interface(ext) => {
            ext.implements_interfaces.is_empty()
                && ext.directives.is_empty()
                && ext.fields.is_empty()
        }
        TypeExtension::Union(ext) => ext.directives.is_empty() && ext.types.is_empty(),
        TypeExtension::Enum(ext) => ext.directives.is_empty() && ext.values.is_empty(),
        TypeExtension::InputObject(ext) => ext.directives.is_empty() && ext.fields.is_empty(),
    };
    (!empty).then_some(extension)
}

// Unlike `schema_type_definition_name`, these do not tie the borrow to the AST
// lifetime, which a trimmed copy of a definition does not live for.
fn type_definition_name<'r>(td: &'r TypeDefinition<'_, String>) -> &'r String {
    match td {
        TypeDefinition::Scalar(scalar_type) => &scalar_type.name,
        TypeDefinition::Object(object_type) => &object_type.name,
        TypeDefinition::Interface(interface_type) => &interface_type.name,
        TypeDefinition::Union(union_type) => &union_type.name,
        TypeDefinition::Enum(enum_type) => &enum_type.name,
        TypeDefinition::InputObject(input_object_type) => &input_object_type.name,
    }
}

/// The name of the type an extension extends.
pub fn type_extension_name<'r>(te: &'r TypeExtension<'_, String>) -> &'r String {
    match te {
        TypeExtension::Scalar(ext) => &ext.name,
        TypeExtension::Object(ext) => &ext.name,
        TypeExtension::Interface(ext) => &ext.name,
        TypeExtension::Union(ext) => &ext.name,
        TypeExtension::Enum(ext) => &ext.name,
        TypeExtension::InputObject(ext) => &ext.name,
    }
}

/// Names of the types and directives schema definitions refer to, gathered
/// by walking every position a name can appear in.
#[derive(Default)]
struct References {
    types: Vec<String>,
    directives: Vec<String>,
}

impl References {
    fn type_definition(&mut self, td: &TypeDefinition<'_, String>) {
        match td {
            TypeDefinition::Scalar(scalar) => self.directives(&scalar.directives),
            TypeDefinition::Object(obj) => {
                self.composite(&obj.implements_interfaces, &obj.directives, &obj.fields)
            }
            TypeDefinition::Interface(iface) => self.composite(
                &iface.implements_interfaces,
                &iface.directives,
                &iface.fields,
            ),
            TypeDefinition::Union(union) => {
                self.directives(&union.directives);
                self.types.extend(union.types.iter().cloned());
            }
            TypeDefinition::Enum(enum_type) => {
                self.directives(&enum_type.directives);
                for value in &enum_type.values {
                    self.directives(&value.directives);
                }
            }
            TypeDefinition::InputObject(input) => {
                self.directives(&input.directives);
                self.input_values(&input.fields);
            }
        }
    }

    fn directive_definition(&mut self, dd: &DirectiveDefinition<'_, String>) {
        self.input_values(&dd.arguments);
    }

    fn composite(
        &mut self,
        interfaces: &[String],
        directives: &[Directive<'_, String>],
        fields: &[Field<'_, String>],
    ) {
        self.types.extend(interfaces.iter().cloned());
        self.directives(directives);
        for field in fields {
            self.named_type(&field.field_type);
            self.directives(&field.directives);
            self.input_values(&field.arguments);
        }
    }

    fn input_values(&mut self, values: &[InputValue<'_, String>]) {
        for value in values {
            self.named_type(&value.value_type);
            self.directives(&value.directives);
        }
    }

    fn named_type(&mut self, ty: &Type<'_, String>) {
        self.types.extend(named_type(ty).cloned());
    }

    fn directives(&mut self, directives: &[Directive<'_, String>]) {
        self.directives
            .extend(directives.iter().map(|directive| directive.name.clone()));
    }
}

/// Asserts that every type and directive `schema` names is defined in it,
/// apart from the built-in scalars and directives. Shared by the tests of every
/// command that emits a schema.
#[cfg(test)]
pub fn assert_self_contained(schema: &str) {
    const BUILT_IN_DIRECTIVES: [&str; 5] =
        ["deprecated", "include", "oneOf", "skip", "specifiedBy"];

    let doc = graphql_parser::parse_schema::<String>(schema)
        .unwrap_or_else(|err| panic!("output is not a parseable schema: {err}\n{schema}"));

    let mut refs = References::default();
    let mut defined_types: HashSet<&str> = BUILT_IN_SCALARS.into_iter().collect();
    let mut defined_directives: HashSet<&str> = BUILT_IN_DIRECTIVES.into_iter().collect();
    for def in &doc.definitions {
        match def {
            Definition::SchemaDefinition(sd) => {
                refs.directives(&sd.directives);
                refs.types.extend(
                    [&sd.query, &sd.mutation, &sd.subscription]
                        .into_iter()
                        .flatten()
                        .cloned(),
                );
            }
            Definition::TypeDefinition(td) => {
                defined_types.insert(type_definition_name(td));
                refs.type_definition(td);
            }
            Definition::TypeExtension(te) => {
                refs.types.push(type_extension_name(te).clone());
                refs.type_definition(&extension_as_definition(te));
            }
            Definition::DirectiveDefinition(dd) => {
                defined_directives.insert(&dd.name);
                refs.directive_definition(dd);
            }
        }
    }

    let undefined = |names: &[String], defined: &HashSet<&str>| {
        let mut missing: Vec<_> = names
            .iter()
            .filter(|name| !defined.contains(name.as_str()))
            .cloned()
            .collect();
        missing.sort();
        missing.dedup();
        missing
    };
    let missing_types = undefined(&refs.types, &defined_types);
    let missing_directives = undefined(&refs.directives, &defined_directives);
    assert!(
        missing_types.is_empty() && missing_directives.is_empty(),
        "schema names undefined types {missing_types:?} and directives {missing_directives:?}:\n{schema}"
    );
}

/// Asserts that no object or interface definition in `schema` is without
/// fields and no union is without members, none of which is valid SDL.
/// Extensions are exempt, since an extension may add only interfaces or
/// directives.
#[cfg(test)]
pub fn assert_no_empty_definitions(schema: &str) {
    let doc = graphql_parser::parse_schema::<String>(schema)
        .unwrap_or_else(|err| panic!("output is not a parseable schema: {err}\n{schema}"));
    let empty: Vec<&String> = doc
        .definitions
        .iter()
        .filter_map(|def| match def {
            Definition::TypeDefinition(td) if has_no_members(td) => Some(type_definition_name(td)),
            _ => None,
        })
        .collect();
    assert!(
        empty.is_empty(),
        "schema has empty definitions {empty:?}:\n{schema}"
    );
}

#[cfg(test)]
mod tests {
    use crate::util;
    use graphql_parser::schema::{InterfaceType, ObjectType, TypeDefinition};
    use indoc::indoc;
    use pretty_assertions::assert_eq;

    /// Merges and prints every type, ordered by name.
    fn merged(schema: &str) -> String {
        let doc = graphql_parser::parse_schema::<String>(schema).unwrap();
        let mut types: Vec<_> = util::merged_type_definitions(&doc).into_iter().collect();
        types.sort_by(|(a, _), (b, _)| a.cmp(b));
        types
            .iter()
            .map(|(_, td)| td.to_string())
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn merges_every_kind_of_extension_into_its_base_type() {
        // Extensions may come before the type they extend.
        let schema = indoc! {"
            extend type User implements Node @key(fields: \"id\") {
              avatar: String
            }

            type User {
              id: ID
            }

            interface Node {
              id: ID
            }

            extend interface Node implements Entity {
              createdAt: String
            }

            interface Entity {
              createdAt: String
            }

            union Result = User

            extend union Result = Bot

            type Bot {
              id: ID
            }

            enum Role {
              ADMIN
            }

            extend enum Role {
              GUEST
            }

            input Filter {
              term: String
            }

            extend input Filter {
              limit: Int
            }

            scalar Date

            extend scalar Date @specifiedBy(url: \"https://example.com\")
        "};

        assert_eq!(
            merged(schema),
            indoc! {"
                type Bot {
                  id: ID
                }

                scalar Date @specifiedBy(url: \"https://example.com\")

                interface Entity {
                  createdAt: String
                }

                input Filter {
                  term: String
                  limit: Int
                }

                interface Node implements Entity {
                  id: ID
                  createdAt: String
                }

                union Result = User | Bot

                enum Role {
                  ADMIN
                  GUEST
                }

                type User implements Node @key(fields: \"id\") {
                  id: ID
                  avatar: String
                }
            "}
        );
    }

    #[test]
    fn treats_an_extension_without_a_base_type_as_the_definition() {
        // The base of `Query` lives in another document, as in a federation
        // subgraph, so its extensions are all there is to go on.
        let schema = indoc! {"
            extend type Query {
              me: User
            }

            extend type Query {
              bots: [User]
            }

            type User {
              id: ID
            }
        "};

        assert_eq!(
            merged(schema),
            indoc! {"
                type Query {
                  me: User
                  bots: [User]
                }

                type User {
                  id: ID
                }
            "}
        );
    }

    #[test]
    fn ignores_an_extension_of_a_different_kind() {
        let schema = indoc! {"
            type User {
              id: ID
            }

            extend union User = Bot

            type Bot {
              id: ID
            }
        "};

        assert_eq!(
            merged(schema),
            indoc! {"
                type Bot {
                  id: ID
                }

                type User {
                  id: ID
                }
            "}
        );
    }

    #[test]
    fn retains_extensions_trimmed_like_their_base_type() {
        let schema = indoc! {"
            type Query {
              me: User
              unused: String
            }

            extend type Query {
              bots: [Bot]
            }

            extend type Query {
              stale: String
            }

            type User {
              id: ID
            }

            type Bot {
              id: ID
            }
        "};
        let doc = graphql_parser::parse_schema::<String>(schema).unwrap();

        // `trim` sees `Query` whole, extensions included, and keeps two of its
        // four fields. The emptied extension goes; the rest keep their layout.
        let result = util::retain_with_dependencies(&doc, &["Query"], &[], |td| match td {
            TypeDefinition::Object(obj) => TypeDefinition::Object(ObjectType {
                fields: obj
                    .fields
                    .iter()
                    .filter(|f| obj.name != "Query" || f.name == "me" || f.name == "bots")
                    .cloned()
                    .collect(),
                ..obj.clone()
            }),
            _ => td.clone(),
        });
        let result = result.to_string();
        util::assert_self_contained(&result);
        util::assert_no_empty_definitions(&result);

        assert_eq!(
            result,
            indoc! {"
                type Query {
                  me: User
                }

                extend type Query {
                  bots: [Bot]
                }

                type User {
                  id: ID
                }

                type Bot {
                  id: ID
                }
            "}
        );
    }

    #[test]
    fn removes_types_trimmed_to_empty_and_every_reference_to_them() {
        let schema = indoc! {"
            type Query {
              me: User
              bot: Bot
              search: [Result]
              machines: [Machine]
              wrapper: Wrapper
            }

            interface Node {
              id: ID
            }

            type User implements Node {
              id: ID
              name: String
            }

            type Bot implements Node {
              id: ID
            }

            union Result = User | Bot

            union Machine = Bot

            type Wrapper {
              bot: Bot
            }
        "};
        let doc = graphql_parser::parse_schema::<String>(schema).unwrap();

        // `trim` empties Bot and Node. Bot goes with the field returning it
        // and its union memberships, which empties Machine and Wrapper in
        // turn, and their fields go too. Node goes from `implements`.
        let result = util::retain_with_dependencies(&doc, &["Query"], &[], |td| match td {
            TypeDefinition::Object(obj) if obj.name == "Bot" => {
                TypeDefinition::Object(ObjectType {
                    fields: vec![],
                    ..obj.clone()
                })
            }
            TypeDefinition::Interface(iface) => TypeDefinition::Interface(InterfaceType {
                fields: vec![],
                ..iface.clone()
            }),
            _ => td.clone(),
        });
        let result = result.to_string();
        util::assert_self_contained(&result);
        util::assert_no_empty_definitions(&result);

        assert_eq!(
            result,
            indoc! {"
                type Query {
                  me: User
                  search: [Result]
                }

                type User {
                  id: ID
                  name: String
                }

                union Result = User
            "}
        );
    }

    #[test]
    fn keeps_definitions_empty_in_the_source_as_written() {
        // Nothing here was emptied by trimming, so kept types stay whole, as
        // `schema focus` relies on.
        let schema = indoc! {"
            type Query

            extend type Query {
              stub: Stub
            }

            type Stub
        "};
        let doc = graphql_parser::parse_schema::<String>(schema).unwrap();

        let result = util::retain_with_dependencies(&doc, &["Query"], &[], |td| td.clone());
        let result = result.to_string();
        util::assert_self_contained(&result);

        assert_eq!(result, schema);
    }
}
