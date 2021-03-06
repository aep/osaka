extern crate proc_macro;
#[macro_use]
extern crate syn;
#[macro_use]
extern crate quote;
extern crate proc_macro2;

use proc_macro::TokenStream;
use syn::export::ToTokens;
use syn::ItemFn;

#[proc_macro_attribute]
pub fn osaka(_args: TokenStream, input: TokenStream) -> TokenStream {
    let mut f = parse_macro_input!(input as ItemFn);

    let output = match f.decl.output.clone() {
        syn::ReturnType::Default => quote! {()},
        syn::ReturnType::Type(_, t) => quote! {#t},
    };

    f.decl.output = match syn::parse((quote! {
        -> osaka::Task<#output>
    }).into(),
    ) {
        Ok(v) => v,
        Err(e) => return e.to_compile_error().into(),
    };

    let oblock = f.block;
    f.block = match syn::parse(
        (quote! {{
            use std::pin::Pin;
            use std::ops::Generator;
            let mut l = Box::new(move||{
                #oblock
            });

            let a = match Pin::new(l.as_mut()).resume() {
                std::ops::GeneratorState::Complete(y) => {
                    return osaka::Task::immediate(y);
                }
                std::ops::GeneratorState::Yielded(y) => {
                    y
                }
            };
            osaka::Task::new(l,a)
        }})
        .into(),
    ) {
        Ok(v) => v,
        Err(e) => return e.to_compile_error().into(),
    };

    f.into_token_stream().into()
}
