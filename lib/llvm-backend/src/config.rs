// Allow unused imports while developing`
#![allow(unused_imports, dead_code)]

use crate::compiler::LLVMCompiler;
use wasmer_compiler::{Compiler, CompilerConfig, CpuFeature, Features, Target};
use inkwell::targets::{Target as LLVMTarget, TargetMachine, TargetTriple, InitializationConfig, CodeModel, RelocMode};
use inkwell::OptimizationLevel;
use itertools::Itertools;
use target_lexicon::Architecture;

#[derive(Clone)]
pub struct LLVMConfig {
    /// Enable NaN canonicalization.
    ///
    /// NaN canonicalization is useful when trying to run WebAssembly
    /// deterministically across different architectures.
    pub enable_nan_canonicalization: bool,

    /// Should the Cranelift verifier be enabled.
    ///
    /// The verifier assures that the generated Cranelift IR is valid.
    pub enable_verifier: bool,

    /// The optimization levels when optimizing the IR.
    pub opt_level: OptimizationLevel,

    features: Features,
    target: Target,
}

impl LLVMConfig {
    /// Creates a new configuration object with the default configuration
    /// specified.
    pub fn new() -> Self {
        Self {
            enable_nan_canonicalization: true,
            enable_verifier: false,
            opt_level: OptimizationLevel::Aggressive,
            features: Default::default(),
            target: Default::default(),
        }
    }
    fn reloc_mode(&self) -> RelocMode {
        RelocMode::Static
    }

    fn code_model(&self) -> CodeModel {
        CodeModel::Large
    }

    /// Generates the target machine for the current target
    pub fn target_machine(&self) -> TargetMachine {
        let target = self.target();
        let triple = target.triple();
        let cpu_features = target.cpu_features().clone();

        match triple.architecture {
            Architecture::X86_64 => LLVMTarget::initialize_x86(&InitializationConfig {
                asm_parser: true,
                asm_printer: true,
                base: true,
                disassembler: true,
                info: true,
                machine_code: true,
            }),
            Architecture::Arm(_) => LLVMTarget::initialize_aarch64(&InitializationConfig {
                asm_parser: true,
                asm_printer: true,
                base: true,
                disassembler: true,
                info: true,
                machine_code: true,
            }),
            _ => unimplemented!("target {} not supported", triple),
        }

        if !cpu_features.contains(CpuFeature::AVX2) {
            panic!("The target needs to support AVX2");
        }

        // The cpu features formatted as LLVM strings
        let llvm_cpu_features = cpu_features.iter().filter_map(|feature| {
            match feature {
                CpuFeature::AVX2 => Some("+avx2"),
                _ => None
            }
        }).join(" ");

        let arch_string = triple.architecture.to_string();
        let llvm_target = LLVMTarget::from_name(&arch_string).unwrap();
        let target_machine = llvm_target.create_target_machine(
            &TargetTriple::create(&target.triple().to_string()),
            &arch_string,
            &llvm_cpu_features,
            self.opt_level.clone(),
            self.reloc_mode(),
            self.code_model(),
        )
        .unwrap();
        target_machine
    }
}

impl CompilerConfig for LLVMConfig {
    /// Gets the WebAssembly features
    fn features(&self) -> &Features {
        &self.features
    }

    /// Gets the WebAssembly features, mutable
    fn features_mut(&mut self) -> &mut Features {
        &mut self.features
    }

    /// Gets the target that we will use for compiling
    /// the WebAssembly module
    fn target(&self) -> &Target {
        &self.target
    }

    /// Gets the target that we will use for compiling
    /// the WebAssembly module, mutable
    fn target_mut(&mut self) -> &mut Target {
        &mut self.target
    }

    /// Transform it into the compiler
    fn compiler(&self) -> Box<dyn Compiler> {
        Box::new(LLVMCompiler::new(&self))
    }
}

impl Default for LLVMConfig {
    fn default() -> LLVMConfig {
        LLVMConfig::new()
    }
}