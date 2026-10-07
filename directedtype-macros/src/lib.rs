use proc_macro::TokenStream;
use quote::quote;
use syn::{parse_macro_input, FnArg, ImplItem, ItemImpl, Pat, ReturnType};

/// Procedural attribute macro for DirectedType companion components.
///
/// Annotate an `impl MyComponent` block with `#[component]`.
/// Any `pub fn <name>(&mut self, ...)` method defined in the block will automatically
/// be registered into an auto-generated `impl Component for MyComponent` with dynamic
/// event dispatching, eliminating the need to write manual string matching or boilerplate error handling.
///
/// Supported method signatures:
/// - `pub fn foo(&mut self)`
/// - `pub fn foo(&mut self, ctx: &mut Context<'_>)`
/// - `pub fn foo(&mut self, event: &mut Event)`
/// - `pub fn foo(&mut self, event: &mut Event, ctx: &mut Context<'_>)`
/// - `pub fn foo(&mut self, ctx: &mut Context<'_>, event: &mut Event)`
/// - Return type can be `()` or `Result<(), DispatchError>`.
///
/// Lifecycle hooks:
/// - `pub fn on_mount(&mut self, ctx: &mut Context<'_>)` is automatically wired to `Component::on_mount`.
#[proc_macro_attribute]
pub fn component(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(item as ItemImpl);
    let self_ty = &input.self_ty;

    let mut match_arms = Vec::new();
    let mut on_mount_impl = None;

    for item in &input.items {
        if let ImplItem::Fn(method) = item {
            // Only process public methods
            if !matches!(method.vis, syn::Visibility::Public(_)) {
                continue;
            }

            let method_name = &method.sig.ident;
            let method_str = method_name.to_string();

            // Check receiver: must take &mut self
            let takes_mut_self = method.sig.inputs.iter().any(|arg| match arg {
                FnArg::Receiver(r) => r.mutability.is_some(),
                _ => false,
            });

            if !takes_mut_self {
                continue;
            }

            // Check if it's on_mount
            if method_str == "on_mount" {
                on_mount_impl = Some(quote! {
                    fn on_mount(&mut self, ctx: &mut directedtype::component::Context<'_>) {
                        self.on_mount(ctx);
                    }
                });
                continue;
            }

            // Process non-receiver arguments
            let other_args: Vec<_> = method
                .sig
                .inputs
                .iter()
                .filter(|arg| !matches!(arg, FnArg::Receiver(_)))
                .collect();

            let call_expr = match other_args.len() {
                0 => quote! { self.#method_name() },
                1 => {
                    let arg = other_args[0];
                    if is_event_type(arg) {
                        quote! { self.#method_name(event) }
                    } else {
                        quote! { self.#method_name(ctx) }
                    }
                }
                2 => {
                    if is_event_type(other_args[0]) {
                        quote! { self.#method_name(event, ctx) }
                    } else {
                        quote! { self.#method_name(ctx, event) }
                    }
                }
                _ => {
                    // Unsupported argument count (> 2)
                    continue;
                }
            };

            let arm_body = match &method.sig.output {
                ReturnType::Default => quote! {
                    {
                        #call_expr;
                        Ok(())
                    }
                },
                ReturnType::Type(_, _) => quote! {
                    #call_expr
                },
            };

            // Support both "foo" and "on_foo" aliases automatically
            let alias_str = if let Some(base) = method_str.strip_prefix("on_") {
                base.to_string()
            } else {
                format!("on_{}", method_str)
            };

            match_arms.push(quote! {
                #method_str | #alias_str => #arm_body,
            });
        }
    }

    let on_mount_tokens = on_mount_impl.unwrap_or_else(|| quote! {});

    let expanded = quote! {
        #input

        impl directedtype::component::Component for #self_ty {
            #on_mount_tokens

            fn dispatch(
                &mut self,
                method: &str,
                event: &mut directedtype::interaction::Event,
                ctx: &mut directedtype::component::Context<'_>,
            ) -> Result<(), directedtype::component::DispatchError> {
                match method {
                    #(#match_arms)*
                    _ => Err(directedtype::component::DispatchError::MethodNotFound {
                        component: stringify!(#self_ty).into(),
                        method: method.into(),
                    }),
                }
            }
        }
    };

    TokenStream::from(expanded)
}

fn is_event_type(arg: &FnArg) -> bool {
    match arg {
        FnArg::Typed(pat_type) => {
            if let Pat::Ident(pat_ident) = &*pat_type.pat {
                if pat_ident.ident == "event" {
                    return true;
                }
            }
            let type_str = quote!(#pat_type).to_string();
            type_str.contains("Event")
        }
        _ => false,
    }
}
