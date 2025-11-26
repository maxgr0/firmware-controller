use crate::util::*;
use proc_macro2::TokenStream;
use quote::quote;
use syn::{spanned::Spanned, Field, Fields, Ident, ItemStruct, LitStr, Result, Token};

/// Information about a published field, to be used by impl processing.
#[derive(Debug, Clone)]
pub(crate) struct PublishedFieldInfo {
    pub field_name: Ident,
    pub subscriber_struct_name: Ident,
}

/// Information about a field with a getter, to be used by impl processing.
#[derive(Debug, Clone)]
pub(crate) struct GetterFieldInfo {
    pub field_name: Ident,
    pub field_type: syn::Type,
    pub getter_name: Ident,
}

/// Information about a field with a public setter, to be used by impl processing.
#[derive(Debug, Clone)]
pub(crate) struct SetterFieldInfo {
    pub field_name: Ident,
    pub field_type: syn::Type,
    /// The public setter method name (client API).
    pub setter_name: Ident,
    /// If the field is published, the internal setter name to call. Otherwise None.
    pub internal_setter_name: Option<Ident>,
}

/// Result of expanding a struct.
pub(crate) struct ExpandedStruct {
    pub tokens: TokenStream,
    pub published_fields: Vec<PublishedFieldInfo>,
    pub getter_fields: Vec<GetterFieldInfo>,
    pub setter_fields: Vec<SetterFieldInfo>,
}

pub(crate) fn expand(mut input: ItemStruct) -> Result<ExpandedStruct> {
    let struct_name = &input.ident;

    let struct_fields = StructFields::parse(&mut input.fields, struct_name)?;
    let field_names = struct_fields.names().collect::<Vec<_>>();

    // Collect published field info.
    let (
        watch_channel_declarations,
        sender_fields_declarations,
        sender_fields_initializations,
        setters,
        subscriber_declarations,
        published_fields_info,
    ) = struct_fields.published().fold(
        (quote!(), quote!(), quote!(), quote!(), quote!(), Vec::new()),
        |(
            watch_channels,
            sender_fields_declarations,
            sender_fields_initializations,
            setters,
            subscribers,
            mut infos,
        ),
         f| {
            let published = f.published.as_ref().unwrap();
            let (watch_channel, sender_field, sender_field_init, setter, subscriber) = (
                &published.watch_channel_declaration,
                &published.sender_field_declaration,
                &published.sender_field_initialization,
                &published.setter,
                &published.subscriber_declaration,
            );

            infos.push(published.info.clone());

            (
                quote! { #watch_channels #watch_channel },
                quote! { #sender_fields_declarations #sender_field, },
                quote! { #sender_fields_initializations #sender_field_init, },
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

    // Collect setter field info.
    let setter_fields_info: Vec<SetterFieldInfo> = struct_fields
        .with_setter()
        .map(|f| {
            let field_name = f.field.ident.as_ref().unwrap().clone();
            let field_type = f.field.ty.clone();
            // Use explicit setter name if provided, otherwise default to set_<field_name>.
            let setter_name =
                f.attrs.setter_name.clone().unwrap_or_else(|| {
                    Ident::new(&format!("set_{}", field_name), field_name.span())
                });
            // If published, use the internal setter; otherwise set field directly.
            let internal_setter_name = if f.attrs.publish {
                Some(Ident::new(
                    &format!("set_{}", field_name),
                    field_name.span(),
                ))
            } else {
                None
            };
            SetterFieldInfo {
                field_name,
                field_type,
                setter_name,
                internal_setter_name,
            }
        })
        .collect();

    let fields = struct_fields.raw_fields().collect::<Vec<_>>();
    let vis = &input.vis;

    // Generate initial value sends for Watch channels.
    let initial_value_sends = published_fields_info.iter().map(|info| {
        let field_name = &info.field_name;
        let sender_name = Ident::new(&format!("{}_sender", field_name), field_name.span());
        quote! {
            __self.#sender_name.send(core::clone::Clone::clone(&__self.#field_name));
        }
    });

    Ok(ExpandedStruct {
        tokens: quote! {
            #vis struct #struct_name {
                #(#fields),*,
                #sender_fields_declarations
            }

            impl #struct_name {
                #[allow(clippy::too_many_arguments)]
                pub fn new(#(#fields),*) -> Self {
                    let __self = Self {
                        #(#field_names),*,
                        #sender_fields_initializations
                    };
                    // Send initial values so subscribers can get them immediately.
                    #(#initial_value_sends)*
                    __self
                }

                #setters
            }

            #watch_channel_declarations

            #subscriber_declarations
        },
        published_fields: published_fields_info,
        getter_fields: getter_fields_info,
        setter_fields: setter_fields_info,
    })
}

/// Parsed controller attributes for a field.
#[derive(Debug, Default)]
struct ControllerAttrs {
    /// Whether the field has `publish` attribute.
    publish: bool,
    /// Whether the field has `pub_setter` (inside publish) - for backwards compatibility.
    pub_setter: bool,
    /// If set, the getter method name (from `getter` or `getter = "name"`).
    getter_name: Option<Ident>,
    /// If set, the setter method name (from `setter` or `setter = "name"`).
    setter_name: Option<Ident>,
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

    /// All fields with setters (either via `setter` attribute or `pub_setter` inside `publish`).
    fn with_setter(&self) -> impl Iterator<Item = &StructField> {
        self.fields
            .iter()
            .filter(|f| f.attrs.setter_name.is_some() || f.attrs.pub_setter)
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
            Some(generate_publish_code(field, struct_name)?)
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
    /// Watch sender field declaration.
    sender_field_declaration: proc_macro2::TokenStream,
    /// Watch sender field initialization.
    sender_field_initialization: proc_macro2::TokenStream,
    /// Field setter.
    setter: proc_macro2::TokenStream,
    /// Watch channel declaration.
    watch_channel_declaration: proc_macro2::TokenStream,
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
        } else if meta.path.is_ident("setter") {
            let field_name = field.ident.as_ref().unwrap();
            if meta.input.peek(Token![=]) {
                meta.input.parse::<Token![=]>()?;
                let name: LitStr = meta.input.parse()?;
                attrs.setter_name = Some(Ident::new(&name.value(), name.span()));
            } else {
                let default_name = format!("set_{}", field_name);
                attrs.setter_name = Some(Ident::new(&default_name, field_name.span()));
            }
        } else {
            let ident = meta.path.get_ident().unwrap();
            let e = format!(
                "expected `publish`, `getter`, or `setter`, found `{}`",
                ident
            );
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

/// Generate code for a published field using Watch channel.
fn generate_publish_code(field: &Field, struct_name: &Ident) -> Result<PublishedFieldCode> {
    let struct_name_str = struct_name.to_string();
    let field_name = field.ident.as_ref().unwrap();
    let field_name_str = field_name.to_string();
    let ty = &field.ty;

    let struct_name_caps = pascal_to_snake_case(&struct_name_str).to_ascii_uppercase();
    let field_name_caps = field_name_str.to_ascii_uppercase();
    let watch_channel_name = Ident::new(
        &format!("{struct_name_caps}_{field_name_caps}_WATCH"),
        field.span(),
    );

    let field_name_pascal = snake_to_pascal_case(&field_name_str);
    let subscriber_struct_name = Ident::new(
        &format!("{struct_name_str}{field_name_pascal}"),
        field.span(),
    );
    let max_subscribers = super::BROADCAST_MAX_SUBSCRIBERS;

    let setter_name = Ident::new(&format!("set_{field_name_str}"), field.span());
    let sender_name = Ident::new(&format!("{field_name_str}_sender"), field.span());

    let sender_field_declaration = quote! {
        #sender_name:
            embassy_sync::watch::Sender<
                'static,
                embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex,
                #ty,
                #max_subscribers,
            >
    };

    let sender_field_initialization = quote! {
        #sender_name: embassy_sync::watch::Watch::sender(&#watch_channel_name)
    };

    // Watch send() is sync, but we keep the setter async for API compatibility.
    let setter = quote! {
        pub async fn #setter_name(&mut self, value: #ty) {
            self.#field_name = value;
            self.#sender_name.send(core::clone::Clone::clone(&self.#field_name));
        }
    };

    let watch_channel_declaration = quote! {
        static #watch_channel_name:
            embassy_sync::watch::Watch<
                embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex,
                #ty,
                #max_subscribers,
            > = embassy_sync::watch::Watch::new();
    };

    let subscriber_declaration = quote! {
        pub struct #subscriber_struct_name {
            receiver: embassy_sync::watch::Receiver<
                'static,
                embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex,
                #ty,
                #max_subscribers,
            >,
            first_poll: bool,
        }

        impl #subscriber_struct_name {
            pub fn new() -> Option<Self> {
                embassy_sync::watch::Watch::receiver(&#watch_channel_name)
                    .map(|receiver| Self {
                        receiver,
                        first_poll: true,
                    })
            }
        }

        impl futures::Stream for #subscriber_struct_name {
            type Item = #ty;

            fn poll_next(
                mut self: core::pin::Pin<&mut Self>,
                cx: &mut core::task::Context<'_>,
            ) -> core::task::Poll<Option<Self::Item>> {
                use core::future::Future;

                let this = self.as_mut().get_mut();

                // First poll: return current value immediately if available.
                if this.first_poll {
                    this.first_poll = false;
                    if let Some(value) = this.receiver.try_get() {
                        return core::task::Poll::Ready(Some(value));
                    }
                }

                // Create changed() future and poll it in place.
                let fut = this.receiver.changed();
                futures::pin_mut!(fut);
                fut.poll(cx).map(Some)
            }
        }
    };

    let info = PublishedFieldInfo {
        field_name: field_name.clone(),
        subscriber_struct_name,
    };

    Ok(PublishedFieldCode {
        sender_field_declaration,
        sender_field_initialization,
        setter,
        watch_channel_declaration,
        subscriber_declaration,
        info,
    })
}
