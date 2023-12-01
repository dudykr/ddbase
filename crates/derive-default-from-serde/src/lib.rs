extern crate proc_macro;

use quote::quote;
use syn::{parse_macro_input, DeriveInput};

/// Derive `Default` from `serde::Deserialize`.
#[proc_macro_derive(SerdeDefault)]
pub fn derive_default_from_serde(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let name = input.ident;
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();

    let expanded = quote! {
        impl #impl_generics ::std::default::Default for #name #ty_generics #where_clause {
            fn default() -> Self {
                let mut deserializer = ::default_from_serde::DefaultDeserializer;
                let t = <Self as ::serde::Deserialize>::deserialize(&mut deserializer).unwrap();
                t
            }
        }
    };

    proc_macro::TokenStream::from(expanded)
}
