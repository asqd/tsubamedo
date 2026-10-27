import React from "react";
import ReactDOM from "react-dom/client";
import "./styles.css";
import App from "./App";
import { isMac } from "./platform";

// The traffic lights sit over the top bar only on a Mac.
if (isMac) document.documentElement.classList.add("mac");

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
