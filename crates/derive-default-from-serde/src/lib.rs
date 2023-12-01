extern crate proc_macro;

use quote::quote;
use syn::{parse_macro_input, DeriveInput};

/// Derive `Default` from `serde::Deserialize`.
#[proc_macro_derive(SerdeDefault)]
pub fn derive_default_from_serde(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let input = parse_macro_input!(input as DeriveInput);

    match &input.data {
        syn::Data::Struct(_) => {}
        syn::Data::Enum(_) => panic!("Enum is not supported"),
        syn::Data::Union(_) => panic!("Union is not supported"),
    }

    let name = input.ident;
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();

    let expanded = quote! {
        impl #impl_generics ::std::default::Default for #name #ty_generics #where_clause {
            fn default() -> Self {
                let  deserializer = ::default_from_serde::DefaultDeserializer;
                let t = <Self as ::serde::Deserialize>::deserialize(deserializer).unwrap();
                t
            }
        }
    };

    proc_macro::TokenStream::from(expanded)
}
