import { render } from "preact";
import "./app.css";

function Mark() {
  return (
    <svg viewBox="0 0 48 48" width="48" height="48" role="img" aria-label="Cairn">
      <rect x="5" y="33" width="38" height="10" rx="5" fill="currentColor" />
      <rect x="10" y="20" width="26" height="10" rx="5" fill="currentColor" />
      <rect x="19" y="8" width="14" height="9" rx="4.5" class="accent" />
    </svg>
  );
}

function App() {
  return (
    <main class="placeholder">
      <Mark />
      <h1>Cairn</h1>
      <p>Your files. One small binary.</p>
    </main>
  );
}

render(<App />, document.getElementById("app")!);
