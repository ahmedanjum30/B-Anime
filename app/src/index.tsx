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
import Pending from "./routes/Pending";
import Settings from "./routes/Settings";

render(
  () => (
    <Router root={Shell}>
      <Route path="/" component={Home} />
      <Route path="/library" component={() => <Library kind="ANIME" />} />
      <Route path="/show/:id" component={() => <Pending title="Show page" />} />
      <Route path="/schedule" component={() => <Pending title="Schedule" />} />
      <Route path="/manga" component={() => <Library kind="MANGA" />} />
      <Route path="/discover" component={() => <Pending title="Discover" />} />
      <Route path="/extensions" component={() => <Pending title="Extensions" />} />
      <Route path="/settings" component={Settings} />
    </Router>
  ),
  document.getElementById("root")!,
);
