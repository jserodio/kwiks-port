#!/usr/bin/env python3
"""Kwiks Launcher — set up and play the modern port of Kwiks.

Made for people who do not want to touch a terminal:

1. Choose the original game (the ZIP you downloaded).
2. Press **Set up and build**. The launcher does the rest.
3. Press **Play**.

Works on Linux and Windows. The heavy lifting lives in :mod:`kwiks_core`.
"""

from __future__ import annotations

import queue
import sys
import threading
import tkinter as tk
from pathlib import Path
from tkinter import filedialog, messagebox, ttk

import kwiks_core as core

APP_TITLE = "Kwiks Launcher"
PAD = 10
OK = "✓"
BAD = "✗"


class Launcher(tk.Tk):
    def __init__(self) -> None:
        super().__init__()
        self.title(APP_TITLE)
        self.minsize(720, 640)
        self.resizable(True, True)

        self._queue: "queue.Queue[str]" = queue.Queue()
        self._busy = False
        self._built_ok = False
        self._result: tuple[str, bool] = ("", False)

        self._build_ui()
        self.refresh_all()
        self.after(100, self._drain_queue)

    # ------------------------------------------------------------------ UI
    def _build_ui(self) -> None:
        outer = ttk.Frame(self, padding=PAD)
        outer.pack(fill="both", expand=True)

        # --- The one button most people need ---------------------------
        hero = ttk.Frame(outer)
        hero.pack(fill="x")
        self.hero_btn = ttk.Button(
            hero,
            text="Set up and build",
            command=self._setup_and_build,
        )
        self.hero_btn.pack(side="left")
        self.hero_status = ttk.Label(hero, text="Press to install the game files and build.", foreground="gray")
        self.hero_status.pack(side="left", padx=(12, 0))

        # --- Step 1: the game files -----------------------------------
        s1 = ttk.LabelFrame(outer, text="1 · The original game (1998)", padding=PAD)
        s1.pack(fill="x", pady=(PAD, 0))
        ttk.Label(
            s1,
            text="Choose the ZIP you downloaded, or the folder where you unzipped the game.",
            wraplength=660,
            justify="left",
        ).grid(row=0, column=0, columnspan=3, sticky="w")
        self.source_var = tk.StringVar()
        ttk.Entry(s1, textvariable=self.source_var).grid(
            row=1, column=0, sticky="ew", padx=(0, 8), pady=(8, 0)
        )
        ttk.Button(s1, text="Choose file…", command=self._pick_file).grid(
            row=1, column=1, pady=(8, 0)
        )
        ttk.Button(s1, text="Choose folder…", command=self._pick_folder).grid(
            row=1, column=2, padx=(8, 0), pady=(8, 0)
        )
        self.files_label = ttk.Label(s1, text="", foreground="gray")
        self.files_label.grid(row=2, column=0, columnspan=3, sticky="w", pady=(8, 0))
        s1.columnconfigure(0, weight=1)

        # --- Step 2: requirements -------------------------------------
        s2 = ttk.LabelFrame(outer, text="2 · What the computer needs", padding=PAD)
        s2.pack(fill="x", pady=(PAD, 0))
        self.deps = ttk.Treeview(
            s2, columns=("status", "detail"), show="tree headings", height=4
        )
        self.deps.heading("#0", text="")
        self.deps.heading("status", text="")
        self.deps.heading("detail", text="")
        self.deps.column("#0", width=230, stretch=False)
        self.deps.column("status", width=34, stretch=False)
        self.deps.column("detail", width=380)
        self.deps.grid(row=0, column=0, columnspan=2, sticky="ew")
        self.deps_hint = ttk.Label(s2, text="", foreground="gray", wraplength=660, justify="left")
        self.deps_hint.grid(row=1, column=0, columnspan=2, sticky="w", pady=(8, 0))
        s2.columnconfigure(0, weight=1)

        # --- Step 3: build & play -------------------------------------
        s3 = ttk.LabelFrame(outer, text="3 · Build and play", padding=PAD)
        s3.pack(fill="x", pady=(PAD, 0))
        self.build_btn = ttk.Button(s3, text="Build the game", command=self._build)
        self.build_btn.grid(row=0, column=0)
        self.play_btn = ttk.Button(s3, text="▶  Play", command=self._play)
        self.play_btn.grid(row=0, column=1, padx=(8, 0))
        self.build_label = ttk.Label(s3, text="", foreground="gray")
        self.build_label.grid(row=0, column=2, sticky="w", padx=(12, 0))
        s3.columnconfigure(2, weight=1)

        # --- Log ------------------------------------------------------
        lf = ttk.LabelFrame(outer, text="Details", padding=PAD)
        lf.pack(fill="both", expand=True, pady=(PAD, 0))
        self.log_view = tk.Text(lf, height=11, wrap="word", state="disabled")
        self.log_view.pack(side="left", fill="both", expand=True)
        sb = ttk.Scrollbar(lf, command=self.log_view.yview)
        sb.pack(side="right", fill="y")
        self.log_view.configure(yscrollcommand=sb.set)

    # ------------------------------------------------------------- logging
    def append_log(self, line: str) -> None:
        """Thread-safe: the worker threads call this, the UI drains the queue."""
        self._queue.put(line)

    def _drain_queue(self) -> None:
        try:
            while True:
                line = self._queue.get_nowait()
                self.log_view.configure(state="normal")
                self.log_view.insert("end", line + "\n")
                self.log_view.see("end")
                self.log_view.configure(state="disabled")
        except queue.Empty:
            pass
        self.after(100, self._drain_queue)

    # ------------------------------------------------------------ refresh
    def refresh_all(self) -> None:
        ok, detail = core.assets_status()
        self.files_label.configure(
            text=(f"{OK} Game files ready — {detail}" if ok else f"{BAD} Game files not installed yet ({detail})"),
            foreground="dark green" if ok else "gray",
        )
        self._refresh_deps()
        self._refresh_build()

    def _refresh_deps(self) -> None:
        self.deps.delete(*self.deps.get_children())
        hints = []
        for d in core.check_dependencies():
            self.deps.insert(
                "",
                "end",
                text=" " + d.name,
                values=(OK if d.ok else BAD, d.detail),
                tags=("ok" if d.ok else "bad"),
            )
            if not d.ok and d.fix_hint:
                hints.append(d.fix_hint)
        self.deps.tag_configure("ok", foreground="dark green")
        self.deps.tag_configure("bad", foreground="dark red")
        self.deps_hint.configure(
            text=" ".join(dict.fromkeys(hints)),
            foreground="dark red" if hints else "gray",
        )

    def _refresh_build(self) -> None:
        binary = core.built_binary()
        self._built_ok = binary is not None
        self.build_label.configure(
            text=(f"{OK} The game is ready to play" if self._built_ok else ""),
            foreground="dark green",
        )
        self.play_btn.configure(state="normal" if self._built_ok else "disabled")

    # ------------------------------------------------------------- actions
    def _pick_file(self) -> None:
        p = filedialog.askopenfilename(
            title="Choose the game ZIP",
            filetypes=[("ZIP files", "*.zip"), ("All files", "*.*")],
        )
        if p:
            self.source_var.set(p)

    def _pick_folder(self) -> None:
        p = filedialog.askdirectory(title="Choose the folder with the game files")
        if p:
            self.source_var.set(p)

    def _run_async(self, job, done=None) -> None:
        if self._busy:
            return
        self._busy = True
        self.hero_btn.configure(state="disabled")
        self.build_btn.configure(state="disabled")

        def worker():
            try:
                job()
            except Exception as e:  # noqa: BLE001 - show the problem, never crash
                self.append_log("")
                self.append_log("Something went wrong: " + str(e))
            finally:
                self._busy = False

                def finish():
                    self.hero_btn.configure(state="normal")
                    self.build_btn.configure(state="normal")
                    if done:
                        done()

                self.after(0, finish)

        threading.Thread(target=worker, daemon=True).start()

    def _extract(self) -> bool:
        src = self.source_var.get().strip()
        if not src:
            self.append_log("Please choose the game first (step 1).")
            return False
        self.append_log(f"Installing the game files from {Path(src).name} …")
        res = core.extract_assets(Path(src), self.append_log)
        for w in res.warnings:
            self.append_log("Warning: " + w)
        ok, detail = core.assets_status()
        self.append_log(f"Game files: {detail}")
        self.refresh_all()
        return ok

    def _setup_and_build(self) -> None:
        """The one button: install the game files, then build."""
        self.hero_status.configure(text="Working… this can take a few minutes.", foreground="gray")
        self._result = ("", False)

        def job():
            self.append_log("=== Setting up Kwiks ===")
            if not core.assets_status()[0] and not self._extract():
                self._result = (f"{BAD} Could not install the game files.", False)
                return
            if core.assets_status()[0]:
                self.append_log("Game files are already installed.")
            good = core.build_project(self.append_log, release=True) == 0
            self._result = (
                (f"{OK} All done — press Play!", True)
                if good
                else (f"{BAD} The build failed. See the details below.", False)
            )

        def done():
            text, ok = self._result
            if text:
                self.hero_status.configure(
                    text=text, foreground="dark green" if ok else "dark red"
                )
            self.refresh_all()

        self._run_async(job, done=done)

    def _build(self) -> None:
        self.build_label.configure(text="Building…", foreground="gray")

        def job():
            self.append_log("=== Building the game ===")
            core.build_project(self.append_log, release=True)

        self._run_async(job, done=self.refresh_all)

    def _play(self) -> None:
        if core.run_game(self.append_log) is None:
            messagebox.showinfo(APP_TITLE, "The game has not been built yet.")
            return
        self.append_log("The game is running. Have fun!")


def diagnose() -> None:
    """Print what the launcher sees. Used for support; no GUI needed."""
    import platform as _platform

    print("Kwiks Launcher — diagnostics")
    print(f"  launcher:   {sys.executable if getattr(sys, 'frozen', False) else __file__}")
    print(f"  system:     {_platform.system()} {_platform.release()} ({_platform.machine()})")
    print(f"  project:    {core.project_root()}")
    print(f"  game src:   {core.game_dir() / 'Cargo.toml'} "
          f"[{'found' if (core.game_dir() / 'Cargo.toml').exists() else 'MISSING'}]")
    ok, detail = core.assets_status()
    print(f"  game files: {'ready' if ok else 'not installed'} ({detail})")
    print("  requirements:")
    for d in core.check_dependencies():
        print(f"    [{'OK' if d.ok else 'MISSING'}] {d.name}: {d.detail}")
        if d.fix_hint:
            print(f"           → {d.fix_hint}")
    missing = core.missing_link_shims()
    print(f"  library links to create: {missing if missing else 'none'}")
    binary = core.built_binary()
    print(f"  built game: {binary if binary else 'not built yet'}")


def setup(source: Optional[str]) -> int:
    """Run the whole flow without a GUI. Returns an exit code."""
    print("=== Setting up Kwiks ===")
    if source:
        print(f"Installing the game files from {Path(source).name} …")
        try:
            core.extract_assets(Path(source), print)
        except Exception as e:  # noqa: BLE001
            print(f"Could not install the game files: {e}")
            return 1
    ok, detail = core.assets_status()
    print(f"Game files: {'ready' if ok else 'not installed'} ({detail})")
    if not ok:
        print("Run again with the path to the game ZIP.")
        return 1
    print("Building the game …")
    code = core.build_project(print, release=True)
    if code == 0:
        print(f"\nAll done. The game is at: {core.built_binary()}")
    return code


def main() -> None:
    args = sys.argv[1:]
    if args and args[0] in ("--diagnose", "-d"):
        diagnose()
        return
    if args and args[0] in ("--setup", "-s"):
        raise SystemExit(setup(args[1] if len(args) > 1 else None))
    Launcher().mainloop()


if __name__ == "__main__":
    main()
