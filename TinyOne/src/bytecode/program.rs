use blake2::{Blake2b512, Digest};

use crate::{Instr, Result, TinyOneError, VmSettings};

/// A host resource that an imported module must be explicitly granted.
///
/// Capabilities are intentionally module-local: calling an imported function
/// from a privileged root program does not lend that program's authority to
/// the module.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ModuleCapability {
    Filesystem,
    Environment,
    Threads,
    UnsafeMemory,
    Network,
    Graphics,
    Hardware,
    LinuxPipelines,
}

impl ModuleCapability {
    const ALL: [Self; 8] = [
        Self::Filesystem,
        Self::Environment,
        Self::Threads,
        Self::UnsafeMemory,
        Self::Network,
        Self::Graphics,
        Self::Hardware,
        Self::LinuxPipelines,
    ];

    pub(crate) const fn bit(self) -> u8 {
        match self {
            Self::Filesystem => 1 << 0,
            Self::Environment => 1 << 1,
            Self::Threads => 1 << 2,
            Self::UnsafeMemory => 1 << 3,
            Self::Network => 1 << 4,
            Self::Graphics => 1 << 5,
            Self::Hardware => 1 << 6,
            Self::LinuxPipelines => 1 << 7,
        }
    }

    pub(crate) const fn name(self) -> &'static str {
        match self {
            Self::Filesystem => "filesystem",
            Self::Environment => "environment",
            Self::Threads => "threads",
            Self::UnsafeMemory => "unsafe_memory",
            Self::Network => "network",
            Self::Graphics => "graphics",
            Self::Hardware => "hardware",
            Self::LinuxPipelines => "linux_pipelines",
        }
    }

    pub(crate) fn parse(name: &str) -> Result<Self> {
        match name {
            "filesystem" => Ok(Self::Filesystem),
            "environment" => Ok(Self::Environment),
            "threads" => Ok(Self::Threads),
            "unsafe_memory" => Ok(Self::UnsafeMemory),
            "network" | "sockets" => Ok(Self::Network),
            "graphics" | "gpu" => Ok(Self::Graphics),
            "hardware" => Ok(Self::Hardware),
            "linux_pipelines" | "pipelines" => Ok(Self::LinuxPipelines),
            _ => Err(TinyOneError::compile(format!(
                "Unknown module capability {name:?}; expected filesystem, environment, threads, unsafe_memory, network (or sockets), graphics (or gpu), hardware, or linux_pipelines"
            ))),
        }
    }
}

/// Compact, serializable set of granted module capabilities.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct ModuleCapabilities(u8);

impl ModuleCapabilities {
    pub(crate) const fn none() -> Self {
        Self(0)
    }

    pub(crate) const fn all() -> Self {
        Self(
            ModuleCapability::Filesystem.bit()
                | ModuleCapability::Environment.bit()
                | ModuleCapability::Threads.bit()
                | ModuleCapability::UnsafeMemory.bit()
                | ModuleCapability::Network.bit()
                | ModuleCapability::Graphics.bit()
                | ModuleCapability::Hardware.bit()
                | ModuleCapability::LinuxPipelines.bit(),
        )
    }

    pub(crate) fn from_names(names: &[String]) -> Result<Self> {
        let mut bits = 0u8;
        for name in names {
            let capability = ModuleCapability::parse(name)?;
            if bits & capability.bit() != 0 {
                return Err(TinyOneError::compile(format!(
                    "Module capability {name:?} is declared more than once"
                )));
            }
            bits |= capability.bit();
        }
        Ok(Self(bits))
    }

    pub(crate) fn from_bits(bits: u8) -> Result<Self> {
        let known = Self::all().0;
        if bits & !known != 0 {
            return Err(TinyOneError::compile(
                "Module capabilities contain unknown bits",
            ));
        }
        Ok(Self(bits))
    }

    pub(crate) const fn allows(self, capability: ModuleCapability) -> bool {
        self.0 & capability.bit() != 0
    }

    pub(crate) const fn bits(self) -> u8 {
        self.0
    }

    pub(crate) const fn intersection(self, other: Self) -> Self {
        Self(self.0 & other.0)
    }

    /// Returns whether every capability in `self` was approved by `other`.
    /// This is used when a signed module's declaration is checked against the
    /// project-level grant before its code is compiled.
    pub(crate) const fn is_subset_of(self, other: Self) -> bool {
        self.0 & !other.0 == 0
    }

    pub(crate) fn names(self) -> Vec<String> {
        ModuleCapability::ALL
            .into_iter()
            .filter(|capability| self.allows(*capability))
            .map(|capability| capability.name().to_string())
            .collect()
    }
}

/// The host authority granted to one compilation unit.
///
/// `ModuleCapabilities` is the compact, backwards-compatible summary used
/// for coarse policy checks.  Signed module manifests are more specific than
/// that summary, though: a module may read files but not write them, and may
/// read only a named set of environment variables.  Keeping those details in
/// the executable program is required so the runtime cannot accidentally turn
/// a signed declaration into a broader grant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ModulePermissions {
    capabilities: ModuleCapabilities,
    filesystem_read: bool,
    filesystem_write: bool,
    /// `None` is the legacy/configured broad environment grant. `Some` is an
    /// exact signed-manifest allowlist, including `Some(vec![])` for no grant.
    environment_read: Option<Vec<String>>,
    network_outbound: bool,
    network_listen: bool,
    process_spawn: bool,
    ffi_allowed: bool,
    graphics_gpu: bool,
    hardware_access: bool,
    threads_allowed: bool,
    unsafe_memory_allowed: bool,
    linux_pipelines_allowed: bool,
}

impl ModulePermissions {
    pub(crate) fn none() -> Self {
        Self::from_capabilities(ModuleCapabilities::none())
    }

    /// Retains the historic semantics of an unqualified Config.toml or
    /// `tinyone.json` capability list: each selected coarse capability grants
    /// all of its currently supported sub-permissions.
    pub(crate) fn from_capabilities(capabilities: ModuleCapabilities) -> Self {
        Self {
            filesystem_read: capabilities.allows(ModuleCapability::Filesystem),
            filesystem_write: capabilities.allows(ModuleCapability::Filesystem),
            environment_read: if capabilities.allows(ModuleCapability::Environment) {
                None
            } else {
                Some(Vec::new())
            },
            network_outbound: capabilities.allows(ModuleCapability::Network),
            network_listen: capabilities.allows(ModuleCapability::Network),
            process_spawn: capabilities.allows(ModuleCapability::LinuxPipelines),
            ffi_allowed: capabilities.allows(ModuleCapability::UnsafeMemory),
            graphics_gpu: capabilities.allows(ModuleCapability::Graphics),
            hardware_access: capabilities.allows(ModuleCapability::Hardware),
            threads_allowed: capabilities.allows(ModuleCapability::Threads),
            unsafe_memory_allowed: capabilities.allows(ModuleCapability::UnsafeMemory),
            linux_pipelines_allowed: capabilities.allows(ModuleCapability::LinuxPipelines),
            capabilities,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn from_signed_manifest(
        filesystem_read: bool,
        filesystem_write: bool,
        environment_read: Vec<String>,
        network_outbound: bool,
        network_listen: bool,
        process_spawn: bool,
        ffi_allowed: bool,
        graphics_gpu: bool,
        hardware_access: bool,
        threads_allowed: bool,
        unsafe_memory_allowed: bool,
        linux_pipelines_allowed: bool,
    ) -> Self {
        let mut bits = 0u8;
        if filesystem_read || filesystem_write {
            bits |= ModuleCapability::Filesystem.bit();
        }
        if !environment_read.is_empty() {
            bits |= ModuleCapability::Environment.bit();
        }
        if threads_allowed {
            bits |= ModuleCapability::Threads.bit();
        }
        if ffi_allowed || unsafe_memory_allowed {
            bits |= ModuleCapability::UnsafeMemory.bit();
        }
        if network_outbound || network_listen {
            bits |= ModuleCapability::Network.bit();
        }
        if graphics_gpu {
            bits |= ModuleCapability::Graphics.bit();
        }
        if hardware_access {
            bits |= ModuleCapability::Hardware.bit();
        }
        if process_spawn || linux_pipelines_allowed {
            bits |= ModuleCapability::LinuxPipelines.bit();
        }
        Self {
            capabilities: ModuleCapabilities(bits),
            filesystem_read,
            filesystem_write,
            environment_read: Some(environment_read),
            network_outbound,
            network_listen,
            process_spawn,
            ffi_allowed,
            graphics_gpu,
            hardware_access,
            threads_allowed,
            unsafe_memory_allowed,
            linux_pipelines_allowed,
        }
    }

    /// Reconstructs policy metadata carried by a versioned artifact.  The
    /// detailed fields must be a subset of the coarse capability bits.
    /// This allows an artifact to retain an intentionally broad legacy grant
    /// while still enforcing its narrower detailed policy at runtime.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn from_artifact(
        capabilities: ModuleCapabilities,
        filesystem_read: bool,
        filesystem_write: bool,
        environment_read: Option<Vec<String>>,
        network_outbound: bool,
        network_listen: bool,
        process_spawn: bool,
        ffi_allowed: bool,
        graphics_gpu: bool,
        hardware_access: bool,
        threads_allowed: bool,
        unsafe_memory_allowed: bool,
        linux_pipelines_allowed: bool,
    ) -> Result<Self> {
        if let Some(allowlist) = &environment_read {
            for (index, name) in allowlist.iter().enumerate() {
                if name.is_empty()
                    || !name.bytes().enumerate().all(|(position, byte)| {
                        byte.is_ascii_uppercase()
                            || byte == b'_'
                            || (position > 0 && byte.is_ascii_digit())
                    })
                    || (index > 0 && allowlist[index - 1] >= *name)
                {
                    return Err(TinyOneError::compile(
                        "Artifact module environment allowlist must contain sorted, unique uppercase variable names",
                    ));
                }
            }
        }
        let expected = Self::from_detailed_fields(
            filesystem_read,
            filesystem_write,
            environment_read
                .as_ref()
                .is_none_or(|items| !items.is_empty()),
            network_outbound,
            network_listen,
            process_spawn,
            ffi_allowed,
            graphics_gpu,
            hardware_access,
            threads_allowed,
            unsafe_memory_allowed,
            linux_pipelines_allowed,
        );
        if !expected.is_subset_of(capabilities) {
            return Err(TinyOneError::compile(
                "Artifact module detailed permissions exceed its declared capabilities",
            ));
        }
        Ok(Self {
            capabilities,
            filesystem_read,
            filesystem_write,
            environment_read,
            network_outbound,
            network_listen,
            process_spawn,
            ffi_allowed,
            graphics_gpu,
            hardware_access,
            threads_allowed,
            unsafe_memory_allowed,
            linux_pipelines_allowed,
        })
    }

    pub(crate) const fn capabilities(&self) -> ModuleCapabilities {
        self.capabilities
    }

    pub(crate) const fn allows_filesystem_read(&self) -> bool {
        self.filesystem_read
    }

    pub(crate) const fn allows_filesystem_write(&self) -> bool {
        self.filesystem_write
    }

    pub(crate) fn allows_environment_read(&self, name: &str) -> bool {
        self.capabilities.allows(ModuleCapability::Environment)
            && self
                .environment_read
                .as_ref()
                .is_none_or(|allowed| allowed.iter().any(|item| item == name))
    }

    pub(crate) const fn network_outbound(&self) -> bool {
        self.network_outbound
    }

    pub(crate) const fn network_listen(&self) -> bool {
        self.network_listen
    }

    pub(crate) const fn process_spawn(&self) -> bool {
        self.process_spawn
    }

    pub(crate) const fn ffi_allowed(&self) -> bool {
        self.ffi_allowed
    }

    pub(crate) const fn graphics_gpu(&self) -> bool {
        self.graphics_gpu
    }

    pub(crate) const fn hardware_access(&self) -> bool {
        self.hardware_access
    }

    pub(crate) const fn threads_allowed(&self) -> bool {
        self.threads_allowed
    }

    pub(crate) const fn unsafe_memory_allowed(&self) -> bool {
        self.unsafe_memory_allowed
    }

    pub(crate) const fn linux_pipelines_allowed(&self) -> bool {
        self.linux_pipelines_allowed
    }

    pub(crate) fn environment_read_allowlist(&self) -> Option<&[String]> {
        self.environment_read.as_deref()
    }

    #[allow(clippy::too_many_arguments)]
    fn from_detailed_fields(
        filesystem_read: bool,
        filesystem_write: bool,
        environment_read: bool,
        network_outbound: bool,
        network_listen: bool,
        process_spawn: bool,
        ffi_allowed: bool,
        graphics_gpu: bool,
        hardware_access: bool,
        threads_allowed: bool,
        unsafe_memory_allowed: bool,
        linux_pipelines_allowed: bool,
    ) -> ModuleCapabilities {
        let mut bits = 0u8;
        if filesystem_read || filesystem_write {
            bits |= ModuleCapability::Filesystem.bit();
        }
        if environment_read {
            bits |= ModuleCapability::Environment.bit();
        }
        if threads_allowed {
            bits |= ModuleCapability::Threads.bit();
        }
        if ffi_allowed || unsafe_memory_allowed {
            bits |= ModuleCapability::UnsafeMemory.bit();
        }
        if network_outbound || network_listen {
            bits |= ModuleCapability::Network.bit();
        }
        if graphics_gpu {
            bits |= ModuleCapability::Graphics.bit();
        }
        if hardware_access {
            bits |= ModuleCapability::Hardware.bit();
        }
        if process_spawn || linux_pipelines_allowed {
            bits |= ModuleCapability::LinuxPipelines.bit();
        }
        ModuleCapabilities(bits)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Function {
    pub(crate) name: String,
    /// Generic parameters are erased at runtime. They are retained in the
    /// program metadata so v2 tooling and artifact consumers can inspect the
    /// declared parametric API.
    pub(crate) generic_params: Vec<String>,
    pub(crate) param_count: usize,
    pub(crate) code: Vec<Instr>,
    pub(crate) slot_count: usize,
    pub(crate) names: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StructDef {
    pub(crate) name: String,
    pub(crate) fields: Vec<String>,
}

/// One flattened, program-global entry per enum variant. `tag` is the
/// variant's 0-based position within its enum's declaration order; `Op::
/// MakeEnum` indexes this table directly by a flat `variant_id`, so no
/// nested enum-name lookup is needed on the execution hot path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnumVariantDef {
    pub enum_name: String,
    pub variant_name: String,
    pub tag: u32,
    pub fields: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleImportDef {
    pub alias: String,
    pub path: String,
    pub module: String,
    pub resolved: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleDef {
    pub(crate) name: String,
    pub(crate) path: String,
    pub(crate) imports: Vec<ModuleImportDef>,
    pub(crate) exported_functions: Vec<String>,
    pub(crate) exported_structs: Vec<String>,
    pub(crate) capabilities: ModuleCapabilities,
    pub(crate) permissions: ModulePermissions,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Program {
    pub(crate) code: Vec<Instr>,
    pub(crate) slot_count: usize,
    pub(crate) names: Vec<String>,
    pub(crate) functions: Vec<Function>,
    pub(crate) strings: Vec<String>,
    pub(crate) structs: Vec<StructDef>,
    pub(crate) fields: Vec<String>,
    pub(crate) modules: Vec<ModuleDef>,
    pub(crate) enum_variants: Vec<EnumVariantDef>,
    /// Authority available to root code. Imported modules retain their own,
    /// narrower grants even when called by root code.
    pub(crate) root_capabilities: ModuleCapabilities,
    pub(crate) root_permissions: ModulePermissions,
    /// Decoded artifacts are data, not an authority grant. This flag is set
    /// only by compiler output or an embedding that explicitly authenticates
    /// an artifact before opting into its serialized policy.
    pub(crate) policy_trusted: bool,
    /// Per-program limits selected by the project configuration.
    pub(crate) vm_settings: VmSettings,
}

impl Program {
    /// Create a program with empty metadata. The resulting program is still
    /// unverified and must pass through `VerifiedProgram` before execution.
    pub fn new(code: Vec<Instr>, slot_count: usize) -> Self {
        Self {
            code,
            slot_count,
            names: Vec::new(),
            functions: Vec::new(),
            strings: Vec::new(),
            structs: Vec::new(),
            fields: Vec::new(),
            modules: Vec::new(),
            enum_variants: Vec::new(),
            root_capabilities: ModuleCapabilities::all(),
            root_permissions: ModulePermissions::from_capabilities(ModuleCapabilities::all()),
            policy_trusted: true,
            vm_settings: VmSettings::default(),
        }
    }

    pub fn code(&self) -> &[Instr] {
        &self.code
    }
    pub fn slot_count(&self) -> usize {
        self.slot_count
    }
    pub fn functions(&self) -> &[Function] {
        &self.functions
    }
    pub fn structs(&self) -> &[StructDef] {
        &self.structs
    }
    pub fn modules(&self) -> &[ModuleDef] {
        &self.modules
    }

    /// Host capabilities granted to the root codebase by `Config.toml`.
    pub fn root_capabilities(&self) -> Vec<String> {
        self.root_capabilities.names()
    }

    /// Maximum nested TinyLang function calls permitted by this program.
    pub fn max_call_depth(&self) -> usize {
        self.vm_settings.max_call_depth
    }

    /// Root code retains the embedding application's authority. Imported
    /// functions execute with only the grants recorded for their owning
    /// module, even when reached through a privileged caller or closure.
    pub(crate) fn capabilities_for_function(
        &self,
        function_index: Option<usize>,
    ) -> ModulePermissions {
        if !self.policy_trusted {
            return ModulePermissions::none();
        }
        let Some(function_index) = function_index else {
            return self.root_permissions.clone();
        };
        let Some(function) = self.functions.get(function_index) else {
            return ModulePermissions::none();
        };
        let Some((module_name, local_name)) = function.name.split_once('.') else {
            return self.root_permissions.clone();
        };
        if local_name.contains('.') {
            return ModulePermissions::none();
        }
        if let Some(module) = self
            .modules
            .iter()
            .find(|module| module.name == module_name)
        {
            module.permissions.clone()
        } else {
            ModulePermissions::none()
        }
    }

    /// Marks policy metadata as an explicit embedding decision after the
    /// artifact bytes have been authenticated by the caller. Generic artifact
    /// loaders intentionally leave it disabled.
    pub(crate) fn trust_artifact_policy(mut self) -> Self {
        self.policy_trusted = true;
        self
    }

    /// Whether this bytecode needs an authenticated cache entry to run.
    /// Disk-cache records are attacker-writable data. Any builtin dispatch is
    /// conservatively treated as host-facing: this avoids making a future
    /// builtin an authority bypass merely because the cache scanner was not
    /// updated at the same time. Such programs are recompiled from current
    /// source and configuration instead.
    pub(crate) fn needs_runtime_host_permissions(&self) -> bool {
        self.code
            .iter()
            .chain(
                self.functions
                    .iter()
                    .flat_map(|function| function.code.iter()),
            )
            .any(|instruction| instruction.op == crate::Op::Builtin)
    }

    pub fn with_functions(mut self, functions: Vec<Function>) -> Self {
        self.functions = functions;
        self
    }

    pub fn with_slot_count(mut self, slot_count: usize) -> Self {
        self.slot_count = slot_count;
        self
    }

    pub fn with_names(mut self, names: Vec<String>) -> Self {
        self.names = names;
        self
    }

    pub fn with_structs(mut self, structs: Vec<StructDef>) -> Self {
        self.structs = structs;
        self
    }

    /// Resolves a string-selected function using the same ownership rules as
    /// bytecode calls. This prevents a module from turning a root function
    /// into a confused deputy through `closure_new` or `thread_spawn`.
    pub(crate) fn callable_function_from(
        &self,
        caller_function: Option<usize>,
        name: &str,
    ) -> Option<(usize, &Function)> {
        let (index, function) = self
            .functions
            .iter()
            .enumerate()
            .find(|(_, function)| function.name == name)?;
        self.can_call_function_from(caller_function, index)
            .then_some((index, function))
    }

    fn can_call_function_from(&self, caller_function: Option<usize>, target: usize) -> bool {
        let caller_module = caller_function.and_then(|index| self.function_module(index));
        let target_module = self.function_module(target);
        match (caller_module, target_module) {
            (None, None) => true,
            (None, Some(target_module)) => self.module_exports_function(target_module, target),
            (Some(_), None) => false,
            (Some(caller_module), Some(target_module)) if caller_module == target_module => true,
            (Some(caller_module), Some(target_module)) => {
                self.module_exports_function(target_module, target)
                    && self.modules[caller_module]
                        .imports
                        .iter()
                        .any(|import| import.resolved == self.modules[target_module].name)
            }
        }
    }

    fn function_module(&self, function_index: usize) -> Option<usize> {
        let function = self.functions.get(function_index)?;
        let (module_name, local_name) = function.name.split_once('.')?;
        if local_name.contains('.') {
            return None;
        }
        self.modules
            .iter()
            .position(|module| module.name == module_name)
    }

    fn module_exports_function(&self, module_index: usize, function_index: usize) -> bool {
        let Some(function) = self.functions.get(function_index) else {
            return false;
        };
        let Some((module_name, local_name)) = function.name.split_once('.') else {
            return false;
        };
        self.modules
            .get(module_index)
            .filter(|module| module.name == module_name)
            .is_some_and(|module| {
                module
                    .exported_functions
                    .iter()
                    .any(|export| export == local_name)
            })
    }

    pub fn fingerprint(&self) -> String {
        let mut hasher = Blake2b512::new();
        hasher.update(b"tinyone-program-fingerprint-v4");
        self.hash_code(&mut hasher, &self.code);
        hasher.update((self.slot_count as u64).to_le_bytes());
        hash_module_permissions(&mut hasher, &self.root_permissions);
        hasher.update((self.vm_settings.max_call_depth as u64).to_le_bytes());
        hash_string_list(&mut hasher, self.names.iter());
        hasher.update((self.functions.len() as u64).to_le_bytes());
        for function in &self.functions {
            hash_string_u32(&mut hasher, &function.name);
            hash_string_list(&mut hasher, function.generic_params.iter());
            hasher.update((function.param_count as u64).to_le_bytes());
            hasher.update((function.slot_count as u64).to_le_bytes());
            self.hash_code(&mut hasher, &function.code);
            hash_string_list(&mut hasher, function.names.iter());
        }
        hasher.update((self.strings.len() as u64).to_le_bytes());
        for text in &self.strings {
            hash_string_u64(&mut hasher, text);
        }
        hasher.update((self.structs.len() as u64).to_le_bytes());
        for item in &self.structs {
            hash_string_u32(&mut hasher, &item.name);
            hasher.update((item.fields.len() as u32).to_le_bytes());
            for field in &item.fields {
                hash_string_u32(&mut hasher, field);
            }
        }
        hash_string_list(&mut hasher, self.fields.iter());
        hasher.update((self.modules.len() as u64).to_le_bytes());
        for module in &self.modules {
            hash_string_u32(&mut hasher, &module.name);
            hash_string_u32(&mut hasher, &module.path);
            hash_string_list(&mut hasher, module.imports.iter().map(|item| &item.alias));
            hash_string_list(&mut hasher, module.imports.iter().map(|item| &item.path));
            hash_string_list(&mut hasher, module.imports.iter().map(|item| &item.module));
            hash_string_list(
                &mut hasher,
                module.imports.iter().map(|item| &item.resolved),
            );
            hash_string_list(&mut hasher, module.exported_functions.iter());
            hash_string_list(&mut hasher, module.exported_structs.iter());
            hash_module_permissions(&mut hasher, &module.permissions);
        }
        hasher.update((self.enum_variants.len() as u64).to_le_bytes());
        for item in &self.enum_variants {
            hash_string_u32(&mut hasher, &item.enum_name);
            hash_string_u32(&mut hasher, &item.variant_name);
            hasher.update(item.tag.to_le_bytes());
            hash_string_list(&mut hasher, item.fields.iter());
        }
        let digest = hasher.finalize();
        hex::encode(&digest[..16])
    }

    fn hash_code(&self, hasher: &mut Blake2b512, code: &[Instr]) {
        hasher.update((code.len() as u64).to_le_bytes());
        for instr in code {
            hasher.update(instr.op.ordinal().to_le_bytes());
            hasher.update((instr.arg as i128).to_le_bytes());
            hasher.update((instr.arg2 as i128).to_le_bytes());
        }
    }
}

impl Function {
    pub fn new(
        name: impl Into<String>,
        param_count: usize,
        code: Vec<Instr>,
        slot_count: usize,
    ) -> Self {
        Self {
            name: name.into(),
            generic_params: Vec::new(),
            param_count,
            code,
            slot_count,
            names: Vec::new(),
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }
    pub fn generic_params(&self) -> &[String] {
        &self.generic_params
    }
    pub fn code(&self) -> &[Instr] {
        &self.code
    }
}

impl StructDef {
    pub fn new(name: impl Into<String>, fields: Vec<String>) -> Self {
        Self {
            name: name.into(),
            fields,
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }
    pub fn fields(&self) -> &[String] {
        &self.fields
    }
}

impl ModuleDef {
    pub fn name(&self) -> &str {
        &self.name
    }
    pub fn path(&self) -> &str {
        &self.path
    }
    pub fn exported_functions(&self) -> &[String] {
        &self.exported_functions
    }
    pub fn exported_structs(&self) -> &[String] {
        &self.exported_structs
    }

    /// Capabilities granted to this imported module by its package manifest.
    pub fn capabilities(&self) -> Vec<String> {
        self.capabilities.names()
    }

    /// Fine-grained signed-manifest declarations, when present. `None` for
    /// environment access means a legacy broad environment capability grant.
    pub fn filesystem_permissions(&self) -> (bool, bool) {
        (
            self.permissions.allows_filesystem_read(),
            self.permissions.allows_filesystem_write(),
        )
    }

    pub fn environment_read_allowlist(&self) -> Option<&[String]> {
        self.permissions.environment_read_allowlist()
    }
}

fn hash_string_u32(hasher: &mut Blake2b512, value: &str) {
    let bytes = value.as_bytes();
    hasher.update((bytes.len() as u32).to_le_bytes());
    hasher.update(bytes);
}

fn hash_string_u64(hasher: &mut Blake2b512, value: &str) {
    let bytes = value.as_bytes();
    hasher.update((bytes.len() as u64).to_le_bytes());
    hasher.update(bytes);
}

fn hash_string_list<'a, I>(hasher: &mut Blake2b512, items: I)
where
    I: ExactSizeIterator<Item = &'a String>,
{
    hasher.update((items.len() as u32).to_le_bytes());
    for item in items {
        hash_string_u32(hasher, item);
    }
}

fn hash_module_permissions(hasher: &mut Blake2b512, permissions: &ModulePermissions) {
    hasher.update([permissions.capabilities.bits()]);
    hasher.update([permissions.filesystem_read as u8]);
    hasher.update([permissions.filesystem_write as u8]);
    match &permissions.environment_read {
        None => hasher.update([0]),
        Some(values) => {
            hasher.update([1]);
            hash_string_list(hasher, values.iter());
        }
    }
    hasher.update([permissions.network_outbound as u8]);
    hasher.update([permissions.network_listen as u8]);
    hasher.update([permissions.process_spawn as u8]);
    hasher.update([permissions.ffi_allowed as u8]);
    hasher.update([permissions.graphics_gpu as u8]);
    hasher.update([permissions.hardware_access as u8]);
    hasher.update([permissions.threads_allowed as u8]);
    hasher.update([permissions.unsafe_memory_allowed as u8]);
    hasher.update([permissions.linux_pipelines_allowed as u8]);
}

/// A `Program` that has been validated by `BytecodeVerifier`.
///
/// Construct via `VerifiedProgram::verify(program)` to guarantee the
/// verification ran. Public execution APIs accept `&VerifiedProgram` or
/// `&Program` (with internal re-verification) — this type is provided for
/// callers that want to verify once and reuse.
#[derive(Debug, Clone)]
pub struct VerifiedProgram {
    program: std::sync::Arc<Program>,
    fingerprint: std::sync::Arc<std::sync::OnceLock<String>>,
}

impl PartialEq for VerifiedProgram {
    fn eq(&self, other: &Self) -> bool {
        self.program == other.program
    }
}

impl Eq for VerifiedProgram {}

impl VerifiedProgram {
    /// Verify `program` and wrap it. Returns `Err` if verification fails.
    pub fn verify(program: Program) -> crate::Result<Self> {
        crate::BytecodeVerifier::verify(&program)?;
        Ok(Self::from_verified_arc(std::sync::Arc::new(program)))
    }

    pub(crate) fn verify_arc(program: std::sync::Arc<Program>) -> crate::Result<Self> {
        crate::BytecodeVerifier::verify(&program)?;
        Ok(Self::from_verified_arc(program))
    }

    pub(crate) fn from_verified_arc(program: std::sync::Arc<Program>) -> Self {
        Self {
            program,
            fingerprint: std::sync::Arc::new(std::sync::OnceLock::new()),
        }
    }

    /// Borrow the inner program.
    pub fn program(&self) -> &Program {
        &self.program
    }

    /// Return the stable program fingerprint, computing it at most once for
    /// this verification token and all of its clones.
    pub fn fingerprint(&self) -> &str {
        self.fingerprint
            .get_or_init(|| self.program.fingerprint())
            .as_str()
    }

    pub(crate) fn program_arc(&self) -> std::sync::Arc<Program> {
        std::sync::Arc::clone(&self.program)
    }

    /// Consume the capability and recover an owned program. If other clones
    /// still share the program, the metadata is cloned for the caller.
    pub fn into_program(self) -> Program {
        match std::sync::Arc::try_unwrap(self.program) {
            Ok(program) => program,
            Err(program) => (*program).clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Op;

    #[test]
    fn cache_authority_scan_rejects_any_builtin_in_main_or_function() {
        let safe = Program::new(vec![Instr::new(Op::Halt, 0, 0)], 0);
        assert!(!safe.needs_runtime_host_permissions());

        let filesystem_builtin = crate::builtin_index("fs_read").expect("filesystem builtin");
        let in_main = Program::new(
            vec![Instr::new(Op::Builtin, filesystem_builtin as i64, 1)],
            0,
        );
        assert!(in_main.needs_runtime_host_permissions());

        let otherwise_safe_builtin = crate::builtin_index("len").expect("length builtin");
        let in_main = Program::new(
            vec![Instr::new(Op::Builtin, otherwise_safe_builtin as i64, 1)],
            0,
        );
        assert!(in_main.needs_runtime_host_permissions());

        let unsafe_builtin = crate::builtin_index("free").expect("unsafe builtin");
        let in_function =
            Program::new(vec![Instr::new(Op::Halt, 0, 0)], 0).with_functions(vec![Function::new(
                "module.release",
                0,
                vec![Instr::new(Op::Builtin, unsafe_builtin as i64, 1)],
                0,
            )]);
        assert!(in_function.needs_runtime_host_permissions());
    }
}
