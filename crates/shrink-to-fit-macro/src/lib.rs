#[proc_macro_derive(ShrinkToFit)]
pub fn derive_shrink_to_fit(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let _input: syn::DeriveInput = syn::parse_macro_input!(input as syn::DeriveInput);

    panic!("todo!")
}
