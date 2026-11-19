//! Android: hands a downloaded, checksum-verified APK to the system package installer.
//!
//! The APK sits in the app's cache folder, shared through the manifest's FileProvider
//! (`<cache-path>` in res/xml/file_paths.xml). Android shows its own "update this app?" screen; it
//! asks first to allow installs from B-Anime (REQUEST_INSTALL_PACKAGES in the manifest).

use std::path::Path;

use jni::JavaVM;
use jni::objects::{JClass, JObject, JValue};

/// Intent flags: FLAG_GRANT_READ_URI_PERMISSION | FLAG_ACTIVITY_NEW_TASK.
const FLAGS: i32 = 0x0000_0001 | 0x1000_0000;

pub fn install_apk(apk: &Path, authority: &str) -> Result<(), String> {
    let ctx = ndk_context::android_context();
    // SAFETY: ndk-context holds the process's JavaVM and the activity, set up by Tauri at launch.
    let vm = unsafe { JavaVM::from_raw(ctx.vm().cast()) }.map_err(|e| e.to_string())?;
    let mut env = vm.attach_current_thread().map_err(|e| e.to_string())?;
    // SAFETY: a global reference owned by ndk-context; never deleted here.
    let context = unsafe { JObject::from_raw(ctx.context().cast()) };

    let result = (|| -> jni::errors::Result<()> {
        let path = env.new_string(apk.to_string_lossy())?;
        let file = env.new_object("java/io/File", "(Ljava/lang/String;)V", &[JValue::Object(&path)])?;

        // FileProvider is an app class: load it through the app's class loader (this thread's
        // default loader only knows framework classes).
        let loader = env.call_method(&context, "getClassLoader", "()Ljava/lang/ClassLoader;", &[])?.l()?;
        let name = env.new_string("androidx.core.content.FileProvider")?;
        let provider: JClass = env
            .call_method(&loader, "loadClass", "(Ljava/lang/String;)Ljava/lang/Class;", &[JValue::Object(&name)])?
            .l()?
            .into();
        let authority = env.new_string(authority)?;
        let uri = env
            .call_static_method(
                &provider,
                "getUriForFile",
                "(Landroid/content/Context;Ljava/lang/String;Ljava/io/File;)Landroid/net/Uri;",
                &[JValue::Object(&context), JValue::Object(&authority), JValue::Object(&file)],
            )?
            .l()?;

        let action = env.new_string("android.intent.action.VIEW")?;
        let intent = env.new_object("android/content/Intent", "(Ljava/lang/String;)V", &[JValue::Object(&action)])?;
        let mime = env.new_string("application/vnd.android.package-archive")?;
        env.call_method(
            &intent,
            "setDataAndType",
            "(Landroid/net/Uri;Ljava/lang/String;)Landroid/content/Intent;",
            &[JValue::Object(&uri), JValue::Object(&mime)],
        )?;
        env.call_method(&intent, "addFlags", "(I)Landroid/content/Intent;", &[JValue::Int(FLAGS)])?;
        env.call_method(&context, "startActivity", "(Landroid/content/Intent;)V", &[JValue::Object(&intent)])?;
        Ok(())
    })();

    // A Java exception must be cleared before this thread talks to the JVM again.
    if env.exception_check().unwrap_or(false) {
        let _ = env.exception_describe();
        let _ = env.exception_clear();
    }
    result.map_err(|e| format!("couldn't open the Android installer: {e}"))
}
