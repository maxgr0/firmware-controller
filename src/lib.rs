#![doc = include_str!("../README.md")]

use proc_macro::TokenStream;
use syn::{parse_macro_input, punctuated::Punctuated, ItemMod, Meta, Token};

mod controller;
mod util;

/// See the crate-level documentation for more information.
#[proc_macro_attribute]
pub fn controller(attr: TokenStream, item: TokenStream) -> TokenStream {
    let _args = parse_macro_input!(attr with Punctuated<Meta, Token![,]>::parse_terminated);

    let input = parse_macro_input!(item as ItemMod);
    controller::expand_module(input)
        .unwrap_or_else(|e| e.to_compile_error())
        .into()
}
