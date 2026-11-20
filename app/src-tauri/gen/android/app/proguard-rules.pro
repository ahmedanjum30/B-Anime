# Add project specific ProGuard rules here.
# You can control the set of applied configuration files using the
# proguardFiles setting in build.gradle.
#
# For more details, see
#   http://developer.android.com/guide/developing/tools/proguard.html

# If your project uses WebView with JS, uncomment the following
# and specify the fully qualified class name to the JavaScript interface
# class:
#-keepclassmembers class fqcn.of.javascript.interface.for.webview {
#   public *;
#}

# Uncomment this to preserve the line number information for
# debugging stack traces.
#-keepattributes SourceFile,LineNumberTable

# If you keep the line number information, uncomment this to
# hide the original source file name.
#-renamesourcefileattribute SourceFile

# The in-app updater (src-tauri/src/android_install.rs) calls FileProvider.getUriForFile by
# name through JNI to hand the downloaded APK to Android's installer. R8 can't see that call,
# so in release builds it inlined/removed the method and installing failed with
# NoSuchMethodError. Keep it exactly as is.
-keep class androidx.core.content.FileProvider {
    public static android.net.Uri getUriForFile(android.content.Context, java.lang.String, java.io.File);
}