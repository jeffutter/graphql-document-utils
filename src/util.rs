use graphql_parser::query::{Directive, Text};
use graphql_parser::schema::{
    Definition, DirectiveDefinition, Document, EnumType, EnumValue, Field, InputObjectType,
    InputValue, InterfaceType, ObjectType, SchemaDefinition, Type, TypeDefinition, TypeExtension,
};
use std::borrow::Borrow;
use std::collections::{HashMap, HashSet};
use std::hash::Hash;

pub fn schema_definition_name<'a, V, D: Text<'a, Value = V>>(
    d: &'a Definition<'a, D>,
) -> Option<&'a V> {
    match d {
        Definition::SchemaDefinition(_) => None,
        Definition::TypeDefinition(type_definition) => schema_type_definition_name(type_definition),
        Definition::TypeExtension(_) => None,
        Definition::DirectiveDefinition(directive_definition) => Some(&directive_definition.name),
    }
}

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

pub fn named_type<'a, V, D: Text<'a, Value = V>>(ty: &'a Type<'a, D>) -> Option<&'a V> {
    match ty {
        Type::NamedType(n) => Some(n),
        Type::ListType(inner) | Type::NonNullType(inner) => named_type(inner),
    }
}

pub trait SchemaWalker<'a, V, D, T = ()>
where
    D: Text<'a, Value = V>,
    V: Hash + Eq + Clone + Borrow<str> + AsRef<str> + 'a,
{
    fn walk_schema(&mut self, doc: &'a Document<'a, D>) {
        let type_map: HashMap<&'a V, &'a TypeDefinition<'a, D>> = doc
            .definitions
            .iter()
            .filter_map(|def| {
                if let Definition::TypeDefinition(ty) = def {
                    Some((schema_type_definition_name(ty).unwrap(), ty))
                } else {
                    None
                }
            })
            .collect();

        let mut interface_map: HashMap<&'a V, HashSet<&'a V>> = HashMap::new();

        for definition in doc.definitions.iter() {
            if let Definition::TypeDefinition(TypeDefinition::Object(obj)) = definition {
                for i in &obj.implements_interfaces {
                    let set = interface_map.entry(i).or_default();
                    set.insert(&obj.name);
                }
            }
        }

        let mut visited = HashSet::new();
        let path = Vec::new();

        for def in &doc.definitions {
            self.walk_definition(def, &type_map, &interface_map, &path, &mut visited);
        }
    }

    fn walk_definition(
        &mut self,
        def: &'a Definition<'a, D>,
        type_map: &HashMap<&'a V, &'a TypeDefinition<'a, D>>,
        interface_map: &HashMap<&'a V, HashSet<&'a V>>,
        path: &Vec<&'a V>,
        visited: &mut HashSet<&'a V>,
    ) {
        match def {
            Definition::SchemaDefinition(schema_def) => {
                self.visit_schema_definition(schema_def, path);
            }
            Definition::TypeDefinition(ty) => {
                let name = schema_type_definition_name(ty).unwrap();
                let mut path = path.clone();
                path.push(name);
                self.visit_type_definition(ty, &path);
                self.walk_type_definition(ty, type_map, interface_map, &path, visited);
            }
            Definition::DirectiveDefinition(dir) => {
                self.visit_directive_definition(dir, path);
            }
            Definition::TypeExtension(ext) => {
                self.visit_type_extension(ext, path);
            }
        }
    }

    fn walk_type_definition(
        &mut self,
        ty: &'a TypeDefinition<'a, D>,
        type_map: &HashMap<&'a V, &'a TypeDefinition<'a, D>>,
        interface_map: &HashMap<&'a V, HashSet<&'a V>>,
        path: &Vec<&'a V>,
        visited: &mut HashSet<&'a V>,
    ) {
        match ty {
            TypeDefinition::Object(obj) => {
                if !visited.insert(&obj.name) {
                    return;
                }

                for field in self.select_object_fields(obj, path) {
                    self.visit_field(field, path);
                    for arg in self.select_arguments(field, path) {
                        self.visit_input_value(arg, path);
                    }
                    for dir in self.select_directives_on_field(field, path) {
                        self.visit_directive(dir, path);
                    }

                    if let Some(type_name) = named_type(&field.field_type) {
                        let mut path = path.clone();
                        path.push(type_name);
                        self.walk_type_by_name(type_name, type_map, interface_map, path, visited);
                    }
                }
                for iface in self.select_implements_interfaces(obj, path) {
                    self.visit_implements_interface(iface, path);
                }
            }
            TypeDefinition::InputObject(input) => {
                if !visited.insert(&input.name) {
                    return;
                }

                for field in self.select_input_fields(input, path) {
                    self.visit_input_value(field, path);

                    if let Some(type_name) = named_type(&field.value_type) {
                        let mut path = path.clone();
                        path.push(type_name);
                        self.walk_type_by_name(type_name, type_map, interface_map, path, visited);
                    }
                }
            }
            TypeDefinition::Interface(interface) => {
                // if !visited.insert(&interface.name) {
                //     return;
                // }

                for field in self.select_interface_fields(interface, path) {
                    self.visit_field(field, path);
                    for arg in self.select_arguments(field, path) {
                        self.visit_input_value(arg, path);
                    }
                    for dir in self.select_directives_on_field(field, path) {
                        self.visit_directive(dir, path);
                    }

                    if let Some(type_name) = named_type(&field.field_type) {
                        let mut path = path.clone();
                        path.push(type_name);
                        self.walk_type_by_name(type_name, type_map, interface_map, path, visited);
                    }
                }

                if let Some(types) = interface_map.get(&interface.name) {
                    for ty in types {
                        self.walk_type_by_name(ty, type_map, interface_map, path.clone(), visited);
                    }
                }
            }
            TypeDefinition::Enum(enum_) => {
                if !visited.insert(&enum_.name) {
                    return;
                }
                for val in self.select_enum_values(enum_, path) {
                    self.visit_enum_value(val, path);
                }
            }
            TypeDefinition::Union(u) => {
                for ty in u.types.iter() {
                    self.walk_type_by_name(ty, type_map, interface_map, path.clone(), visited);
                }
            }
            TypeDefinition::Scalar(_) => {
                // Scalars and Unions don't have fields to walk.
            }
        }
    }

    fn walk_type_by_name(
        &mut self,
        name: &'a V,
        type_map: &HashMap<&'a V, &'a TypeDefinition<'a, D>>,
        interface_map: &HashMap<&'a V, HashSet<&'a V>>,
        path: Vec<&'a V>,
        visited: &mut HashSet<&'a V>,
    ) {
        if !visited.insert(name) {
            return;
        }

        if let Some(ty) = type_map.get(name) {
            let mut path = path.clone();
            path.push(name);
            self.visit_type_definition(ty, &path);
            self.walk_type_definition(ty, type_map, interface_map, &path, visited);
        }

        visited.remove(name);
    }

    // --- Visit hooks ---

    fn visit_schema_definition(&mut self, _def: &'a SchemaDefinition<'a, D>, _path: &[&'a V]) {}
    fn visit_type_definition(&mut self, _ty: &'a TypeDefinition<'a, D>, _path: &[&'a V]) {}
    fn visit_directive_definition(
        &mut self,
        _dir: &'a DirectiveDefinition<'a, D>,
        _path: &[&'a V],
    ) {
    }
    fn visit_type_extension(&mut self, _ext: &'a TypeExtension<'a, D>, _path: &[&'a V]) {}
    fn visit_field(&mut self, _field: &'a Field<'a, D>, _path: &[&'a V]) {}
    fn visit_input_value(&mut self, _input: &'a InputValue<'a, D>, _path: &[&'a V]) {}
    fn visit_directive(&mut self, _dir: &'a Directive<'a, D>, _path: &[&'a V]) {}
    fn visit_enum_value(&mut self, _val: &'a EnumValue<'a, D>, _path: &[&'a V]) {}
    fn visit_implements_interface(&mut self, _iface: V, _path: &[&'a V]) {}

    // --- Child selectors (default: all) ---

    fn select_object_fields(
        &mut self,
        obj: &'a ObjectType<'a, D>,
        _path: &[&'a V],
    ) -> Vec<&'a Field<'a, D>> {
        obj.fields.iter().collect()
    }

    fn select_interface_fields(
        &mut self,
        iface: &'a InterfaceType<'a, D>,
        _path: &[&'a V],
    ) -> Vec<&'a Field<'a, D>> {
        iface.fields.iter().collect()
    }

    fn select_input_fields(
        &mut self,
        input: &'a InputObjectType<'a, D>,
        _path: &[&'a V],
    ) -> Vec<&'a InputValue<'a, D>> {
        input.fields.iter().collect()
    }

    fn select_enum_values(
        &mut self,
        enum_: &'a EnumType<'a, D>,
        _path: &[&'a V],
    ) -> Vec<&'a EnumValue<'a, D>> {
        enum_.values.iter().collect()
    }

    fn select_arguments(
        &mut self,
        field: &'a Field<'a, D>,
        _path: &[&'a V],
    ) -> Vec<&'a InputValue<'a, D>> {
        field.arguments.iter().collect()
    }

    fn select_directives_on_field(
        &mut self,
        field: &'a Field<'a, D>,
        _path: &[&'a V],
    ) -> Vec<&'a Directive<'a, D>> {
        field.directives.iter().collect()
    }

    fn select_implements_interfaces(
        &mut self,
        obj: &'a ObjectType<'a, D>,
        _path: &[&'a V],
    ) -> Vec<V> {
        obj.implements_interfaces.clone()
    }
}
