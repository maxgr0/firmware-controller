use crate::util::*;
use proc_macro2::TokenStream;
use quote::quote;
use syn::{spanned::Spanned, Field, Fields, Ident, ItemStruct, LitStr, Result, Token};

/// Information about a published field, to be used by impl processing.
#[derive(Debug, Clone)]
pub(crate) struct PublishedFieldInfo {
    pub field_name: Ident,
    pub field_type: syn::Type,
    pub setter_name: Ident,
    pub subscriber_struct_name: Ident,
    pub pub_setter: bool,
}

/// Information about a field with a getter, to be used by impl processing.
#[derive(Debug, Clone)]
pub(crate) struct GetterFieldInfo {
    pub field_name: Ident,
    pub field_type: syn::Type,
    pub getter_name: Ident,
}

/// Result of expanding a struct.
pub(crate) struct ExpandedStruct {
    pub tokens: TokenStream,
    pub published_fields: Vec<PublishedFieldInfo>,
    pub getter_fields: Vec<GetterFieldInfo>,
}

pub(crate) fn expand(mut input: ItemStruct) -> Result<ExpandedStruct> {
    let struct_name = &input.ident;

    let struct_fields = StructFields::parse(&mut input.fields, struct_name)?;
    let field_names = struct_fields.names().collect::<Vec<_>>();

    // Collect published field info.
    let (
        publish_channel_declarations,
        publisher_fields_declarations,
        publisher_fields_initializations,
        setters,
        subscriber_declarations,
        published_fields_info,
    ) = struct_fields.published().fold(
        (quote!(), quote!(), quote!(), quote!(), quote!(), Vec::new()),
        |(
            publish_channels,
            publisher_fields_declarations,
            publisher_fields_initializations,
            setters,
            subscribers,
            mut infos,
        ),
         f| {
            let published = f.published.as_ref().unwrap();
            let (publish_channel, publisher_field, publisher_field_init, setter, subscriber) = (
                &published.publish_channel_declaration,
                &published.publisher_field_declaration,
                &published.publisher_field_initialization,
                &published.setter,
                &published.subscriber_declaration,
            );

            infos.push(published.info.clone());

            (
                quote! { #publish_channels #publish_channel },
                quote! { #publisher_fields_declarations #publisher_field, },
                quote! { #publisher_fields_initializations #publisher_field_init, },
                quote! { #setters #setter },
                quote! { #subscribers #subscriber },
                infos,
            )
        },
    );

    // Collect getter field info.
    let getter_fields_info: Vec<GetterFieldInfo> = struct_fields
        .with_getter()
        .map(|f| {
            let field_name = f.field.ident.as_ref().unwrap().clone();
            let field_type = f.field.ty.clone();
            let getter_name = f.attrs.getter_name.clone().unwrap();
            GetterFieldInfo {
                field_name,
                field_type,
                getter_name,
            }
        })
        .collect();

    let fields = struct_fields.raw_fields().collect::<Vec<_>>();
    let vis = &input.vis;

    Ok(ExpandedStruct {
        tokens: quote! {
            #vis struct #struct_name {
                #(#fields),*,
                #publisher_fields_declarations
            }

            impl #struct_name {
                #[allow(clippy::too_many_arguments)]
                pub fn new(#(#fields),*) -> Self {
                    Self {
                        #(#field_names),*,
                        #publisher_fields_initializations
                    }
                }

                #setters
            }

            #publish_channel_declarations

            #subscriber_declarations
        },
        published_fields: published_fields_info,
        getter_fields: getter_fields_info,
    })
}

/// Parsed controller attributes for a field.
#[derive(Debug, Default)]
struct ControllerAttrs {
    /// Whether the field has `publish` attribute.
    publish: bool,
    /// Whether the field has `pub_setter` (inside publish).
    pub_setter: bool,
    /// If set, the getter method name (from `getter` or `getter = "name"`).
    getter_name: Option<Ident>,
}

/// Parsed struct fields.
#[derive(Debug)]
struct StructFields {
    fields: Vec<StructField>,
}

impl StructFields {
    /// Parse the fields of the struct.
    fn parse(fields: &mut Fields, struct_name: &Ident) -> Result<Self> {
        let fields = match fields {
            Fields::Named(fields) => fields
                .named
                .iter_mut()
                .map(|field| StructField::parse(field, struct_name))
                .collect::<Result<Vec<_>>>()?,
            Fields::Unnamed(_) | Fields::Unit => {
                return Err(syn::Error::new_spanned(
                    fields,
                    "controller struct must have only named fields",
                ))
            }
        };

        Ok(Self { fields })
    }

    /// Names of all the fields.
    fn names(&self) -> impl Iterator<Item = &syn::Ident> {
        self.fields.iter().map(|f| f.field.ident.as_ref().unwrap())
    }

    /// All raw fields.
    fn raw_fields(&self) -> impl Iterator<Item = &Field> {
        self.fields.iter().map(|f| &f.field)
    }

    /// All the published fields.
    fn published(&self) -> impl Iterator<Item = &StructField> {
        self.fields.iter().filter(|f| f.published.is_some())
    }

    /// All fields with getters.
    fn with_getter(&self) -> impl Iterator<Item = &StructField> {
        self.fields.iter().filter(|f| f.attrs.getter_name.is_some())
    }
}

/// A struct field with its parsed controller attributes and generated code.
#[derive(Debug)]
struct StructField {
    /// The field with controller attributes removed.
    field: Field,
    /// Parsed controller attributes.
    attrs: ControllerAttrs,
    /// Generated publish code (if `publish` attribute is present).
    published: Option<PublishedFieldCode>,
}

impl StructField {
    /// Parse a struct field.
    fn parse(field: &mut Field, struct_name: &Ident) -> Result<Self> {
        let attrs = parse_controller_attrs(field)?;

        let published = if attrs.publish {
            Some(generate_publish_code(field, struct_name, attrs.pub_setter)?)
        } else {
            None
        };

        Ok(Self {
            field: field.clone(),
            attrs,
            published,
        })
    }
}

/// Generated code for a published field.
#[derive(Debug)]
struct PublishedFieldCode {
    /// Publisher field declaration.
    publisher_field_declaration: proc_macro2::TokenStream,
    /// Publisher field initialization.
    publisher_field_initialization: proc_macro2::TokenStream,
    /// Field setter.
    setter: proc_macro2::TokenStream,
    /// Publish channel declaration.
    publish_channel_declaration: proc_macro2::TokenStream,
    /// Subscriber struct declaration.
    subscriber_declaration: proc_macro2::TokenStream,
    /// Information to be passed to impl processing.
    info: PublishedFieldInfo,
}

/// Parse the `#[controller(...)]` attributes from a field.
fn parse_controller_attrs(field: &mut Field) -> Result<ControllerAttrs> {
    let mut attrs = ControllerAttrs::default();

    let Some(attr) = field
        .attrs
        .iter()
        .find(|attr| attr.path().is_ident("controller"))
    else {
        return Ok(attrs);
    };

    attr.parse_nested_meta(|meta| {
        if meta.path.is_ident("publish") {
            attrs.publish = true;

            // Parse nested attributes like `publish(pub_setter)`.
            if meta.input.peek(syn::token::Paren) {
                let content;
                syn::parenthesized!(content in meta.input);
                while !content.is_empty() {
                    let nested_ident: Ident = content.parse()?;
                    if nested_ident == "pub_setter" {
                        attrs.pub_setter = true;
                    } else {
                        let e = format!("expected `pub_setter`, found `{}`", nested_ident);
                        return Err(syn::Error::new_spanned(&nested_ident, e));
                    }

                    if !content.is_empty() {
                        content.parse::<Token![,]>()?;
                    }
                }
            }
        } else if meta.path.is_ident("getter") {
            let field_name = field.ident.as_ref().unwrap();
            if meta.input.peek(Token![=]) {
                meta.input.parse::<Token![=]>()?;
                let name: LitStr = meta.input.parse()?;
                attrs.getter_name = Some(Ident::new(&name.value(), name.span()));
            } else {
                attrs.getter_name = Some(field_name.clone());
            }
        } else {
            let ident = meta.path.get_ident().unwrap();
            let e = format!("expected `publish` or `getter`, found `{}`", ident);
            return Err(syn::Error::new_spanned(ident, e));
        }

        Ok(())
    })?;

    // Remove controller attributes from the field.
    field
        .attrs
        .retain(|attr| !attr.path().is_ident("controller"));

    Ok(attrs)
}

/// Generate code for a published field.
fn generate_publish_code(
    field: &Field,
    struct_name: &Ident,
    pub_setter: bool,
) -> Result<PublishedFieldCode> {
    let struct_name_str = struct_name.to_string();
    let field_name = field.ident.as_ref().unwrap();
    let field_name_str = field_name.to_string();
    let ty = &field.ty;

    let struct_name_caps = pascal_to_snake_case(&struct_name_str).to_ascii_uppercase();
    let field_name_caps = field_name_str.to_ascii_uppercase();
    let publish_channel_name = Ident::new(
        &format!("{struct_name_caps}_{field_name_caps}_CHANNEL"),
        field.span(),
    );

    let field_name_pascal = snake_to_pascal_case(&field_name_str);
    let subscriber_struct_name = Ident::new(
        &format!("{struct_name_str}{field_name_pascal}"),
        field.span(),
    );
    let change_struct_name = Ident::new(
        &format!("{struct_name_str}{field_name_pascal}Changed"),
        field.span(),
    );
    let capacity = super::ALL_CHANNEL_CAPACITY;
    let max_subscribers = super::BROADCAST_MAX_SUBSCRIBERS;
    let max_publishers = super::BROADCAST_MAX_PUBLISHERS;

    let setter_name = Ident::new(&format!("set_{field_name_str}"), field.span());
    let publisher_name = Ident::new(&format!("{field_name_str}_publisher"), field.span());
    let publisher_field_declaration = quote! {
        #publisher_name:
            embassy_sync::pubsub::Publisher<
                'static,
                embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex,
                #change_struct_name,
                #capacity,
                #max_subscribers,
                #max_publishers,
            >
    };
    let publisher_field_initialization = quote! {
        // We only create one publisher so we can't fail.
        #publisher_name: embassy_sync::pubsub::PubSubChannel::publisher(&#publish_channel_name).unwrap()
    };
    let setter = quote! {
        pub async fn #setter_name(&mut self, mut value: #ty) {
            core::mem::swap(&mut self.#field_name, &mut value);

            let change = #change_struct_name {
                previous: value,
                new: core::clone::Clone::clone(&self.#field_name),
            };
            embassy_sync::pubsub::publisher::Pub::publish_immediate(
                &self.#publisher_name,
                change,
            );
        }
    };

    let publish_channel_declaration = quote! {
        static #publish_channel_name:
            embassy_sync::pubsub::PubSubChannel<
                embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex,
                #change_struct_name,
                #capacity,
                #max_subscribers,
                #max_publishers,
            > = embassy_sync::pubsub::PubSubChannel::new();
    };

    let subscriber_declaration = quote! {
        pub struct #subscriber_struct_name {
            subscriber: embassy_sync::pubsub::Subscriber<
                'static,
                embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex,
                #change_struct_name,
                #capacity,
                #max_subscribers,
                #max_publishers,
            >,
        }

        impl #subscriber_struct_name {
            pub fn new() -> Option<Self> {
                embassy_sync::pubsub::PubSubChannel::subscriber(&#publish_channel_name)
                    .ok()
                    .map(|subscriber| Self { subscriber })
            }
        }

        impl futures::Stream for #subscriber_struct_name {
            type Item = #change_struct_name;

            fn poll_next(
                self: core::pin::Pin<&mut Self>,
                cx: &mut core::task::Context<'_>,
            ) -> core::task::Poll<Option<Self::Item>> {
                let subscriber = core::pin::Pin::new(&mut *self.get_mut().subscriber);
                futures::Stream::poll_next(subscriber, cx)
            }
        }

        #[derive(Debug, Clone)]
        pub struct #change_struct_name {
            pub previous: #ty,
            pub new: #ty,
        }
    };

    let info = PublishedFieldInfo {
        field_name: field_name.clone(),
        field_type: ty.clone(),
        setter_name,
        subscriber_struct_name,
        pub_setter,
    };

    Ok(PublishedFieldCode {
        publisher_field_declaration,
        publisher_field_initialization,
        setter,
        publish_channel_declaration,
        subscriber_declaration,
        info,
    })
}
