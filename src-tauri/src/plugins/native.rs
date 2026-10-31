//! Native Plugin API 2 dispatcher. Shared libraries and APKs use the same lifecycle/UI contract.
use super::{
    model::{native_target, NativePackage, Package},
    PluginState,
};
use player_plugin_sdk::{Buffer, Host, HostApi, NativePlugin, Result as PluginResult};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    ffi::{c_void, CStr},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex, OnceLock,
    },
    time::Duration,
};
use tauri::{Emitter, Manager};

type Create = unsafe extern "C" fn() -> *mut c_void;
type Destroy = unsafe extern "C" fn(*mut c_void);
type Call = unsafe extern "C" fn(*mut c_void, *const u8, usize, HostApi) -> Buffer;
type Free = unsafe extern "C" fn(Buffer);
struct LibraryInstance {
    _library: Arc<libloading::Library>,
    instance: *mut c_void,
    call: Call,
    free: Free,
    destroy: Destroy,
}
// Access to the ABI instance is serialized by its activation mutex.
unsafe impl Send for LibraryInstance {}
impl Drop for LibraryInstance {
    fn drop(&mut self) {
        unsafe { (self.destroy)(self.instance) }
    }
}
enum Backend {
    Builtin(Box<dyn NativePlugin>),
    Library(LibraryInstance),
    Android,
}
struct Activation {
    generation: u64,
    backend: Backend,
    initialized: bool,
}
#[derive(Default)]
pub struct NativeRuntime {
    activations: Mutex<BTreeMap<String, Arc<Mutex<Activation>>>>,
    closing: AtomicBool,
    // Keep library code resident until shutdown, even after disabling an instance.
    libraries: Mutex<Vec<Arc<libloading::Library>>>,
}
#[derive(Clone)]
struct Context {
    app: tauri::AppHandle,
    id: String,
    generation: u64,
}
struct Reply {
    id: String,
    generation: u64,
    send: std::sync::mpsc::Sender<PluginResult>,
}
static NEXT: AtomicU64 = AtomicU64::new(1);
static REPLIES: OnceLock<Mutex<BTreeMap<u64, Reply>>> = OnceLock::new();
fn replies() -> &'static Mutex<BTreeMap<u64, Reply>> {
    REPLIES.get_or_init(Default::default)
}
#[cfg(target_os = "android")]
static ANDROID_CONTEXTS: OnceLock<Mutex<BTreeMap<u64, Context>>> = OnceLock::new();
impl Context {
    fn active(&self) -> Result<(), String> {
        if self
            .app
            .state::<NativeRuntime>()
            .closing
            .load(Ordering::Acquire)
        {
            return Err("Native runtime is closing".into());
        }
        let state = self.app.state::<PluginState>();
        let record = state.record(&self.id)?;
        if !record.enabled || record.generation != self.generation {
            return Err("DISABLED: plugin activation ended".into());
        }
        if !state
            .manifest(&self.id)?
            .platforms
            .iter()
            .any(|p| p == super::model::platform())
        {
            return Err("Unsupported native plugin platform".into());
        }
        Ok(())
    }
}
impl Host for Context {
    fn call(&self, method: &str, args: Value) -> PluginResult {
        self.active()?;
        if method == "host.invalidate" {
            let _ = self.app.emit(
                "plugin-invalidated",
                json!({"pluginId":self.id,"generation":self.generation}),
            );
            return Ok(Value::Null);
        }
        if method == "host.background" {
            let topic = args["topic"]
                .as_str()
                .ok_or("Missing background topic")?
                .to_string();
            let task_id = args["id"].clone();
            let method = args["method"]
                .as_str()
                .ok_or("Missing background method")?
                .to_string();
            if topic.len() > 100 || method.len() > 100 {
                return Err("Invalid background request".into());
            }
            let context = self.clone();
            let task_args = args["args"].clone();
            tauri::async_runtime::spawn(async move {
                let result = super::plugin_call(
                    context.id.clone(),
                    context.generation,
                    method,
                    task_args,
                    context.app.clone(),
                )
                .await;
                let payload = match result {
                    Ok(value) => json!({"id":task_id,"result":value}),
                    Err(error) => json!({"id":task_id,"error":error,"result":null}),
                };
                super::emit_generation_event(
                    &context.app,
                    &context.id,
                    context.generation,
                    &topic,
                    payload,
                );
            });
            return Ok(Value::Null);
        }
        if method.starts_with("host.") {
            let id = NEXT.fetch_add(1, Ordering::Relaxed);
            let (send, receive) = std::sync::mpsc::channel();
            replies().lock().map_err(|e| e.to_string())?.insert(
                id,
                Reply {
                    id: self.id.clone(),
                    generation: self.generation,
                    send,
                },
            );
            let emitted=self.app.emit("plugin-host-call",json!({"requestId":id,"pluginId":self.id,"generation":self.generation,"method":method,"args":args}));
            let result = if let Err(e) = emitted {
                Err(e.to_string())
            } else {
                receive
                    .recv_timeout(Duration::from_secs(45))
                    .map_err(|_| "Native host request timed out".to_string())
                    .and_then(|v| v)
            };
            replies().lock().map_err(|e| e.to_string())?.remove(&id);
            self.active()?;
            return result;
        }
        tauri::async_runtime::block_on(super::plugin_call(
            self.id.clone(),
            self.generation,
            method.into(),
            args,
            self.app.clone(),
        ))
    }
}
unsafe extern "C" fn host_call(context: *mut c_void, request: *const u8, len: usize) -> Buffer {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> PluginResult {
        if context.is_null() || request.is_null() || len > player_plugin_sdk::MAX_MESSAGE_BYTES {
            return Err("Invalid host request".into());
        }
        let value: Value = serde_json::from_slice(std::slice::from_raw_parts(request, len))
            .map_err(|e| e.to_string())?;
        (&*context.cast::<Context>()).call(
            value["method"].as_str().ok_or("Missing host method")?,
            value["args"].clone(),
        )
    }))
    .unwrap_or_else(|_| Err("Native host callback panicked".into()));
    Buffer::new(player_plugin_sdk::encode(result))
}
fn library(
    app: &tauri::AppHandle,
    package: &Package,
    fingerprint: &str,
) -> Result<Backend, String> {
    let NativePackage::Library { artifacts } = &package.native else {
        return Err("Missing native artifacts".into());
    };
    let bytes = artifacts
        .get(&native_target())
        .ok_or("No native library for this platform and architecture")?
        .decode()?;
    let root = app
        .state::<PluginState>()
        .root
        .join("native")
        .join(&package.manifest.id)
        .join(fingerprint);
    std::fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    let path = root.join(format!("plugin.{}", std::env::consts::DLL_EXTENSION));
    if std::fs::read(&path).ok().as_deref() != Some(&bytes) {
        super::store::write_atomic(&path, &bytes)?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))
            .map_err(|e| e.to_string())?;
    }
    // No library is opened during preview or installation: only after explicit enable.
    unsafe {
        let library = Arc::new(
            libloading::Library::new(&path)
                .map_err(|e| format!("Could not load native plugin: {e}"))?,
        );
        let version = *library
            .get::<unsafe extern "C" fn() -> u32>(b"player_plugin_abi_version\0")
            .map_err(|e| e.to_string())?;
        if version() != player_plugin_sdk::ABI_VERSION {
            return Err("Incompatible native ABI".into());
        }
        let identity = *library
            .get::<unsafe extern "C" fn() -> *const std::ffi::c_char>(b"player_plugin_id\0")
            .map_err(|e| e.to_string())?;
        let id = identity();
        if id.is_null() || CStr::from_ptr(id).to_bytes() != package.manifest.id.as_bytes() {
            return Err("Native plugin ID mismatch".into());
        }
        let create = *library
            .get::<Create>(b"player_plugin_create\0")
            .map_err(|e| e.to_string())?;
        let call = *library
            .get::<Call>(b"player_plugin_call\0")
            .map_err(|e| e.to_string())?;
        let free = *library
            .get::<Free>(b"player_plugin_free\0")
            .map_err(|e| e.to_string())?;
        let destroy = *library
            .get::<Destroy>(b"player_plugin_destroy\0")
            .map_err(|e| e.to_string())?;
        let instance = create();
        if instance.is_null() {
            return Err("Native plugin constructor failed".into());
        }
        app.state::<NativeRuntime>()
            .libraries
            .lock()
            .map_err(|e| e.to_string())?
            .push(library.clone());
        Ok(Backend::Library(LibraryInstance {
            _library: library,
            instance,
            call,
            free,
            destroy,
        }))
    }
}
fn backend(
    app: &tauri::AppHandle,
    package: &Package,
    fingerprint: &str,
) -> Result<Backend, String> {
    match &package.native {
        NativePackage::Library { .. } => library(app, package, fingerprint),
        NativePackage::AndroidApk => Ok(Backend::Android),
        NativePackage::Builtin => match package.manifest.id.as_str() {
            #[cfg(feature = "discord-rpc")]
            "player.discord-rpc" => Ok(Backend::Builtin(Box::new(
                player_discord_plugin::Plugin::default(),
            ))),
            #[cfg(feature = "soulseek")]
            "player.soulseek" => Ok(Backend::Builtin(Box::new(
                player_soulseek_plugin::Plugin::default(),
            ))),
            _ => Err("Unknown native builtin".into()),
        },
    }
}
fn dispatch(
    activation: &mut Activation,
    context: &mut Context,
    method: &str,
    args: Value,
) -> PluginResult {
    match &mut activation.backend {
        Backend::Builtin(plugin) => plugin.call(method, args, context),
        Backend::Library(plugin) => unsafe {
            let request = serde_json::to_vec(&json!({"method":method,"args":args}))
                .map_err(|e| e.to_string())?;
            let result = (plugin.call)(
                plugin.instance,
                request.as_ptr(),
                request.len(),
                HostApi {
                    context: (context as *mut Context).cast(),
                    call: host_call,
                    free: Buffer::free,
                },
            );
            let value = result.read().and_then(player_plugin_sdk::decode);
            (plugin.free)(result);
            value
        },
        Backend::Android => {
            #[cfg(target_os = "android")]
            {
                let binding = context
                    .app
                    .state::<PluginState>()
                    .package(&context.id)?
                    .manifest
                    .android_native
                    .ok_or("Missing APK binding")?;
                let token = NEXT.fetch_add(1, Ordering::Relaxed);
                ANDROID_CONTEXTS
                    .get_or_init(Default::default)
                    .lock()
                    .map_err(|e| e.to_string())?
                    .insert(token, context.clone());
                let response = super::android::mobile_call(
                    "call",
                    json!({"pluginId":context.id,"generation":context.generation,"binding":binding,"method":method,"argsJson":args.to_string(),"hostToken":token}),
                );
                ANDROID_CONTEXTS
                    .get_or_init(Default::default)
                    .lock()
                    .map_err(|e| e.to_string())?
                    .remove(&token);
                response.map(|v| v["result"].clone())
            }
            #[cfg(not(target_os = "android"))]
            {
                Err("APK plugins require Android".into())
            }
        }
    }
}
#[tauri::command]
pub async fn plugin_native_call(
    app: tauri::AppHandle,
    plugin_id: String,
    generation: u64,
    method: String,
    args: Value,
) -> PluginResult {
    if ![
        "activate",
        "render",
        "action",
        "search",
        "renderSearch",
        "playback",
        "event",
    ]
    .contains(&method.as_str())
    {
        return Err("Unknown native operation".into());
    }
    if serde_json::to_vec(&args).map_err(|e| e.to_string())?.len()
        > player_plugin_sdk::MAX_MESSAGE_BYTES
    {
        return Err("Native request exceeds 1 MiB".into());
    }
    tauri::async_runtime::spawn_blocking(move || {
        let mut context = Context {
            app: app.clone(),
            id: plugin_id.clone(),
            generation,
        };
        context.active()?;
        let runtime = app.state::<NativeRuntime>();
        let activation = {
            let mut activations = runtime.activations.lock().map_err(|e| e.to_string())?;
            if let Some(existing) = activations.get(&plugin_id) {
                existing.clone()
            } else {
                if method != "activate" {
                    return Err("Native plugin is not active".into());
                }
                let package = app.state::<PluginState>().package(&plugin_id)?;
                let fingerprint = package.fingerprint();
                let entry = Arc::new(Mutex::new(Activation {
                    generation,
                    backend: backend(&app, &package, &fingerprint)?,
                    initialized: false,
                }));
                activations.insert(plugin_id.clone(), entry.clone());
                entry
            }
        };
        let mut activation = activation.lock().map_err(|e| e.to_string())?;
        context.active()?;
        if activation.generation != generation {
            return Err("Stale native activation; disable and enable the plugin".into());
        }
        if method == "activate" && activation.initialized {
            return Ok(Value::Null);
        }
        let result = dispatch(&mut activation, &mut context, &method, args);
        if method == "activate" && result.is_ok() {
            activation.initialized = true;
        }
        context.active()?;
        result
    })
    .await
    .map_err(|e| e.to_string())?
}
pub fn stop(app: &tauri::AppHandle, id: &str, generation: u64) {
    if let Ok(mut pending) = replies().lock() {
        pending.retain(|_, reply| {
            if reply.id == id {
                let _ = reply.send.send(Err("CANCELLED: plugin disabled".into()));
                false
            } else {
                true
            }
        });
    }

    let runtime = app.state::<NativeRuntime>();
    let activation = runtime
        .activations
        .lock()
        .ok()
        .and_then(|mut a| a.remove(id));
    if let Some(activation) = activation {
        if let Ok(mut activation) = activation.lock() {
            let mut context = Context {
                app: app.clone(),
                id: id.into(),
                generation,
            };
            if !matches!(activation.backend, Backend::Android)
                || !runtime.closing.load(Ordering::Acquire)
            {
                let _ = dispatch(&mut activation, &mut context, "deactivate", json!({}));
            }
        }
    }
}
pub fn shutdown(app: &tauri::AppHandle) {
    let runtime = app.state::<NativeRuntime>();
    runtime.closing.store(true, Ordering::Release);
    let ids: Vec<_> = runtime
        .activations
        .lock()
        .map(|map| map.keys().cloned().collect())
        .unwrap_or_default();
    for id in ids {
        let generation = app
            .state::<PluginState>()
            .record(&id)
            .map(|r| r.generation)
            .unwrap_or(0);
        stop(app, &id, generation);
    }
}

#[tauri::command]
pub fn plugin_native_reply(
    request_id: u64,
    plugin_id: String,
    generation: u64,
    result: Option<Value>,
    error: Option<String>,
) -> Result<(), String> {
    let mut pending = replies().lock().map_err(|e| e.to_string())?;
    let Some(reply) = pending.get(&request_id) else {
        return Ok(());
    };
    if reply.id != plugin_id || reply.generation != generation {
        return Err("Invalid native callback owner".into());
    }
    let reply = pending.remove(&request_id).unwrap();
    let _ = reply.send.send(match error {
        Some(e) => Err(e),
        None => Ok(result.unwrap_or(Value::Null)),
    });
    Ok(())
}
#[cfg(target_os = "android")]
#[no_mangle]
pub extern "system" fn Java_dev_verncat_player_NativeHostBridge_call(
    mut env: jni::JNIEnv,
    _class: jni::objects::JClass,
    token: jni::sys::jlong,
    method: jni::objects::JString,
    args: jni::objects::JString,
) -> jni::sys::jstring {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> PluginResult {
        let method: String = env.get_string(&method).map_err(|e| e.to_string())?.into();
        let args: String = env.get_string(&args).map_err(|e| e.to_string())?.into();
        if args.len() > player_plugin_sdk::MAX_MESSAGE_BYTES {
            return Err("Host request exceeds 1 MiB".into());
        }
        let context = ANDROID_CONTEXTS
            .get_or_init(Default::default)
            .lock()
            .map_err(|e| e.to_string())?
            .get(&(token as u64))
            .cloned()
            .ok_or("Expired native context")?;
        context.call(
            &method,
            serde_json::from_str(&args).map_err(|e| e.to_string())?,
        )
    }))
    .unwrap_or_else(|_| Err("Android host callback panicked".into()));
    let response = String::from_utf8(player_plugin_sdk::encode(result)).unwrap_or_default();
    env.new_string(response)
        .map(|s| s.into_raw())
        .unwrap_or(std::ptr::null_mut())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn compiled_plugin_runs_through_c_abi_and_calls_host_without_an_interpreter() {
        let manifest =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../plugins/Cargo.toml");
        let target = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../plugins/target");
        let output = std::process::Command::new("cargo")
            .args(["build", "--locked", "--offline", "--manifest-path"])
            .arg(&manifest)
            .args(["-p", "player-hello-plugin"])
            .env("CARGO_TARGET_DIR", &target)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let path = target.join("debug").join(format!(
            "{}player_hello_plugin.{}",
            std::env::consts::DLL_PREFIX,
            std::env::consts::DLL_EXTENSION
        ));
        let packaged = std::env::temp_dir().join(format!(
            "player-native-abi-{}.player-plugin",
            std::process::id()
        ));
        let package_source = manifest.parent().unwrap().join("examples/hello");
        let output = std::process::Command::new("cargo")
            .args(["run", "--locked", "--offline", "--manifest-path"])
            .arg(&manifest)
            .args([
                "-p",
                "player-plugin-sdk",
                "--bin",
                "player-plugin-pack",
                "--",
            ])
            .arg(package_source)
            .arg(&packaged)
            .arg(format!("{}={}", native_target(), path.display()))
            .env("CARGO_TARGET_DIR", &target)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let package = Package::parse(&std::fs::read_to_string(&packaged).unwrap(), false).unwrap();
        assert_eq!(package.manifest.api_version, 2);
        assert_eq!(package.manifest.id, "example.hello");
        std::fs::remove_file(packaged).unwrap();
        unsafe extern "C" fn callback(
            context: *mut c_void,
            request: *const u8,
            len: usize,
        ) -> Buffer {
            let state = &mut *context.cast::<Value>();
            let request: Value =
                serde_json::from_slice(std::slice::from_raw_parts(request, len)).unwrap();
            let result = match request["method"].as_str().unwrap() {
                "settings.get" => Ok(state.clone()),
                "settings.update" => {
                    *state = json!({"revision":1,"values":request["args"]["values"]});
                    Ok(state.clone())
                }
                "host.invalidate" => Ok(Value::Null),
                _ => Err("Unexpected host method".into()),
            };
            Buffer::new(player_plugin_sdk::encode(result))
        }
        unsafe {
            let library = libloading::Library::new(path).unwrap();
            let version = library
                .get::<unsafe extern "C" fn() -> u32>(b"player_plugin_abi_version\0")
                .unwrap();
            assert_eq!(version(), 1);
            let identity = library
                .get::<unsafe extern "C" fn() -> *const std::ffi::c_char>(b"player_plugin_id\0")
                .unwrap();
            assert_eq!(
                CStr::from_ptr(identity()).to_str().unwrap(),
                "example.hello"
            );
            let create = library.get::<Create>(b"player_plugin_create\0").unwrap();
            let call = library.get::<Call>(b"player_plugin_call\0").unwrap();
            let free = library.get::<Free>(b"player_plugin_free\0").unwrap();
            let destroy = library.get::<Destroy>(b"player_plugin_destroy\0").unwrap();
            let instance = create();
            assert!(!instance.is_null());
            let mut settings = json!({"revision":0,"values":{}});
            let mut invoke = |method: &str, args: Value| {
                let request = json!({"method":method,"args":args}).to_string();
                let buffer = call(
                    instance,
                    request.as_ptr(),
                    request.len(),
                    HostApi {
                        context: (&mut settings as *mut Value).cast(),
                        call: callback,
                        free: Buffer::free,
                    },
                );
                let value = player_plugin_sdk::decode(buffer.read().unwrap());
                free(buffer);
                value
            };
            invoke("activate", json!({})).unwrap();
            invoke(
                "action",
                json!({"action":"save","fields":{"greeting":"Native ABI works"}}),
            )
            .unwrap();
            assert_eq!(
                invoke("render", json!({"viewId":"home"})).unwrap()["title"],
                "Native ABI works"
            );
            assert!(invoke("unknown", json!({})).is_err());
            invoke("deactivate", json!({})).unwrap();
            destroy(instance);
        }
    }
    #[test]
    fn callback_reply_is_bound_to_its_owner_and_generation() {
        let (send, receive) = std::sync::mpsc::channel();
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        replies().lock().unwrap().insert(
            id,
            Reply {
                id: "example.a".into(),
                generation: 5,
                send,
            },
        );
        assert!(plugin_native_reply(id, "example.b".into(), 5, Some(json!(1)), None).is_err());
        assert!(plugin_native_reply(id, "example.a".into(), 4, Some(json!(1)), None).is_err());
        plugin_native_reply(id, "example.a".into(), 5, Some(json!(1)), None).unwrap();
        assert_eq!(receive.recv().unwrap().unwrap(), json!(1));
    }
}
