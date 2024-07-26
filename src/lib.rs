use std::{
    ffi::{OsStr, OsString},
    path::Path,
};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum OptLevel {
    No,
    Less,
    #[default]
    Default,
    Size,
    SizeMin,
    Aggressive,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum LinkOutputKind {
    /// Dynamically linked non position-independent executable.
    DynamicNoPicExe,
    /// Dynamically linked position-independent executable.
    DynamicPicExe,
    /// Statically linked non position-independent executable.
    StaticNoPicExe,
    /// Statically linked position-independent executable.
    StaticPicExe,
    /// Regular dynamic library ("dynamically linked").
    DynamicDylib,
    /// Dynamic library with bundled libc ("statically linked").
    StaticDylib,
    /// WASI module with a lifetime past the _initialize entry point
    WasiReactorExe,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Strip {
    None,
    DebugInfo,
    Symbols,
}

pub struct GnuLikeLinker {
    pub args: Vec<OsString>,
}

impl GnuLikeLinker {
    pub fn hint_static(&mut self) {
        self.link_arg("-Bstatic");
    }

    pub fn hint_dynamic(&mut self) {
        self.link_arg("-Bdynamic");
    }

    pub fn hint_symbolic(&mut self) {
        self.link_arg("-Bsymbolic");
    }

    pub fn link_arg(&mut self, arg: impl AsRef<OsStr>) -> &mut Self {
        self.args.push(arg.as_ref().to_os_string());
        self
    }

    pub fn link_args(
        &mut self,
        args: impl IntoIterator<Item: AsRef<OsStr>, IntoIter: ExactSizeIterator>,
    ) {
        for arg in args.into_iter() {
            self.args.push(arg.as_ref().to_os_string());
        }
    }

    pub fn push_linker_plugin_lto_args(
        &mut self,
        plugin_path: Option<&OsStr>,
        opt_level: OptLevel,
        profile_sample_path: Option<&Path>,
        target_cpu: &str,
    ) {
        if let Some(plugin_path) = plugin_path {
            let mut arg = OsString::from("-plugin=");
            arg.push(plugin_path);
            self.link_arg(&arg);
        }

        let opt_level = match opt_level {
            OptLevel::No => "O0",
            OptLevel::Less => "O1",
            OptLevel::Default | OptLevel::Size | OptLevel::SizeMin => "O2",
            OptLevel::Aggressive => "O3",
        };

        if let Some(path) = &profile_sample_path {
            self.link_arg(&format!("-plugin-opt=sample-profile={}", path.display()));
        };
        self.link_arg(&format!("-plugin-opt={opt_level}"));
        self.link_arg(&format!("-plugin-opt=mcpu={}", target_cpu));
    }

    pub fn build_dylib(&mut self, out_filename: &Path) {
        self.link_arg("-shared");

        if let Some(name) = out_filename.file_name() {
            // When dylibs are linked by a full path this value will get into `DT_NEEDED`
            // instead of the full path, so the library can be later found in some other
            // location than that specific path.
            let mut soname = OsString::from("-soname=");
            soname.push(name);
            self.link_arg(soname);
        }
    }

    pub fn with_as_needed(&mut self, as_needed: bool, f: impl FnOnce(&mut Self)) {
        if !as_needed {
            self.link_arg("--no-as-needed");
        }

        f(self);

        if !as_needed {
            self.link_arg("--as-needed");
        }
    }

    pub fn set_output_kind(&mut self, output_kind: LinkOutputKind, out_filename: &Path) {
        match output_kind {
            LinkOutputKind::DynamicNoPicExe => {
                self.link_arg("-no-pie");
            }
            LinkOutputKind::DynamicPicExe => {
                self.link_arg("-pie");
            }
            LinkOutputKind::StaticNoPicExe => {
                // `-static` works for both gcc wrapper and ld.
                self.link_arg("-static");
                self.link_arg("-no-pie");
            }
            LinkOutputKind::StaticPicExe => {
                // `--no-dynamic-linker` and `-z text` are not strictly necessary for producing
                // a static pie, but currently passed because gcc and clang pass them.
                // The former suppresses the `INTERP` ELF header specifying dynamic linker,
                // which is otherwise implicitly injected by ld (but not lld).
                // The latter doesn't change anything, only ensures that everything is pic.
                self.link_args(&["-static", "-pie", "--no-dynamic-linker", "-z", "text"]);
            }
            LinkOutputKind::DynamicDylib => self.build_dylib(out_filename),
            LinkOutputKind::StaticDylib => {
                self.link_arg("-static");
                self.build_dylib(out_filename);
            }
            LinkOutputKind::WasiReactorExe => {
                self.link_args(&["--entry", "_initialize"]);
            }
        }
    }

    pub fn link_dylib_by_name(&mut self, name: &str, verbatim: bool, as_needed: bool) {
        self.hint_dynamic();
        self.with_as_needed(as_needed, |this| {
            let colon = if verbatim { ":" } else { "" };
            this.link_arg(format!("-l{colon}{name}"));
        });
    }

    pub fn link_dylib_by_path(&mut self, path: &Path, as_needed: bool) {
        self.hint_dynamic();
        self.with_as_needed(as_needed, |this| {
            this.link_arg(path);
        })
    }

    pub fn link_staticlib_by_name(&mut self, name: &str, verbatim: bool, whole_archive: bool) {
        self.hint_static();
        let colon = if verbatim { ":" } else { "" };
        if !whole_archive {
            self.link_arg(format!("-l{colon}{name}"));
        } else {
            self.link_arg("--whole-archive")
                .link_arg(format!("-l{colon}{name}"))
                .link_arg("--no-whole-archive");
        }
    }

    pub fn link_staticlib_by_path(&mut self, path: &Path, whole_archive: bool) {
        self.hint_static();
        if !whole_archive {
            self.link_arg(path);
        } else {
            self.link_arg("--whole-archive")
                .link_arg(path)
                .link_arg("--no-whole-archive");
        }
    }

    pub fn framework_path(&mut self, path: &Path) {
        self.link_arg("-F").link_arg(path);
    }

    pub fn full_relro(&mut self) {
        self.link_args(&["-z", "relro", "-z", "now"]);
    }

    pub fn partial_relro(&mut self) {
        self.link_args(&["-z", "relro"]);
    }

    pub fn no_relro(&mut self) {
        self.link_args(&["-z", "norelro"]);
    }

    pub fn gc_sections(&mut self, keep_metadata: bool) {
        // If we're building a dylib, we don't use --gc-sections because LLVM
        // has already done the best it can do, and we also don't want to
        // eliminate the metadata. If we're building an executable, however,
        // --gc-sections drops the size of hello world from 1.8MB to 597K, a 67%
        // reduction.
        if !keep_metadata {
            self.link_arg("--gc-sections");
        }
    }

    pub fn no_gc_sections(&mut self) {
        self.link_arg("--no-gc-sections");
    }

    pub fn optimize(&mut self) {
        // GNU-style linkers support optimization with -O. GNU ld doesn't
        // need a numeric argument, but other linkers do.
        self.link_arg("-O1");
    }

    pub fn pgo_gen(&mut self) {
        // If we're doing PGO generation stuff and on a GNU-like linker, use the
        // "-u" flag to properly pull in the profiler runtime bits.
        //
        // This is because LLVM otherwise won't add the needed initialization
        // for us on Linux (though the extra flag should be harmless if it
        // does).
        //
        // See https://reviews.llvm.org/D14033 and https://reviews.llvm.org/D14030.
        //
        // Though it may be worth to try to revert those changes upstream, since
        // the overhead of the initialization should be minor.
        self.link_args(&["-u", "__llvm_profile_runtime"]);
    }

    pub fn debuginfo(&mut self, strip: Strip) {
        match strip {
            Strip::None => {}
            Strip::DebugInfo => {
                self.link_arg("--strip-debug");
            }
            Strip::Symbols => {
                self.link_arg("--strip-all");
            }
        }
    }

    pub fn add_eh_frame_header(&mut self) {
        self.link_arg("--eh-frame-hdr");
    }

    pub fn add_no_exec(&mut self) {
        self.link_args(&["-z", "noexecstack"]);
    }

    pub fn add_as_needed(&mut self) {
        self.link_arg("--as-needed");
    }
}
