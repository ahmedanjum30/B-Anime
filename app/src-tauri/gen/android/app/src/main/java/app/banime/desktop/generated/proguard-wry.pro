# THIS FILE IS AUTO-GENERATED. DO NOT MODIFY!!

# Copyright 2020-2023 Tauri Programme within The Commons Conservancy
# SPDX-License-Identifier: Apache-2.0
# SPDX-License-Identifier: MIT

-keep class app.banime.desktop.* {
  native <methods>;
}

-keep class app.banime.desktop.WryActivity {
  public <init>(...);

  void setWebView(app.banime.desktop.RustWebView);
  java.lang.Class getAppClass(...);
  int getId();
  java.lang.String getVersion();
  int startActivity(...);
}

-keep class app.banime.desktop.Ipc {
  public <init>(...);

  @android.webkit.JavascriptInterface public <methods>;
}

-keep class app.banime.desktop.RustWebView {
  public <init>(...);

  void loadUrlMainThread(...);
  void loadHTMLMainThread(...);
  void evalScript(...);
  java.lang.String getCookies(java.lang.String);
  void clearAllBrowsingData();
}

-keep class app.banime.desktop.RustWebChromeClient,app.banime.desktop.RustWebViewClient {
  public <init>(...);
}
