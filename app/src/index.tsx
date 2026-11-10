/* @refresh reload */
import { lazy } from "solid-js";
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

// Home ships in the first bundle; every other screen loads on first visit.
const Library = lazy(() => import("./routes/Library"));
const Show = lazy(() => import("./routes/Show"));
const Schedule = lazy(() => import("./routes/Schedule"));
const Discover = lazy(() => import("./routes/Discover"));
const Extensions = lazy(() => import("./routes/Extensions"));
const Player = lazy(() => import("./routes/Player"));
const Reader = lazy(() => import("./routes/Reader"));
const Settings = lazy(() => import("./routes/Settings"));

render(
  () => (
    <Router root={Shell}>
      <Route path="/" component={Home} />
      <Route path="/library" component={() => <Library kind="ANIME" />} />
      <Route path="/manga" component={() => <Library kind="MANGA" />} />
      <Route path="/show/:id" component={Show} />
      <Route path="/watch/:id/:ep" component={Player} />
      <Route path="/read/:id/:ch" component={Reader} />
      <Route path="/schedule" component={Schedule} />
      <Route path="/discover" component={Discover} />
      <Route path="/extensions" component={Extensions} />
      <Route path="/settings" component={Settings} />
    </Router>
  ),
  document.getElementById("root")!,
);
