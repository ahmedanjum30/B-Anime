/* @refresh reload */
import { render } from "solid-js/web";
import { Router, Route } from "@solidjs/router";

import "@fontsource/archivo/400.css";
import "@fontsource/archivo/500.css";
import "@fontsource/archivo/600.css";
import "@fontsource/archivo-narrow/600.css";
import "@fontsource/archivo-narrow/700.css";
import "./styles/tokens.css";
import "./styles/base.css";
import "./styles/app.css";

import Shell from "./components/Shell";
import Home from "./routes/Home";
import Library from "./routes/Library";
import Show from "./routes/Show";
import Schedule from "./routes/Schedule";
import Discover from "./routes/Discover";
import Extensions from "./routes/Extensions";
import Settings from "./routes/Settings";

render(
  () => (
    <Router root={Shell}>
      <Route path="/" component={Home} />
      <Route path="/library" component={() => <Library kind="ANIME" />} />
      <Route path="/show/:id" component={Show} />
      <Route path="/schedule" component={Schedule} />
      <Route path="/manga" component={() => <Library kind="MANGA" />} />
      <Route path="/discover" component={Discover} />
      <Route path="/extensions" component={Extensions} />
      <Route path="/settings" component={Settings} />
    </Router>
  ),
  document.getElementById("root")!,
);
