//! Typed Tauri commands that forward one request to the Host.

/// Keep the command's public signature and ACL while sharing its Host call.
/// The identifier before the function names its Tauri state parameter.
macro_rules! host_command {
    (
        $state:ident;
        $(#[$attribute:meta])*
        $visibility:vis async fn $name:ident($($argument:tt)*) -> $output:ty { $($request:tt)* }
    ) => {
        $(#[$attribute])*
        #[tauri::command]
        $visibility async fn $name($($argument)*) -> $output {
            let request = { $($request)* };
            $crate::host_call::call(&$state, request).await
        }
    };
}
