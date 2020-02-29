use crate::{abi, intrinsics::Intrinsics};
use inkwell::{
    attributes::{Attribute, AttributeLoc},
    builder::Builder,
    context::Context,
    module::{Linkage, Module},
    types::{BasicType, BasicTypeEnum, FunctionType},
    values::FunctionValue,
    AddressSpace,
};
use wasmer_runtime_core::{
    module::ModuleInfo,
    structures::{SliceMap, TypedIndex},
    types::{FuncSig, SigIndex, Type},
};

pub fn generate_trampolines<'ctx>(
    info: &ModuleInfo,
    signatures: &SliceMap<SigIndex, (FunctionType<'ctx>, Vec<(Attribute, AttributeLoc)>)>,
    module: &Module<'ctx>,
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    intrinsics: &Intrinsics<'ctx>,
) -> Result<(), String> {
    for (sig_index, sig) in info.signatures.iter() {
        let (func_type, func_attrs) = &signatures[sig_index];

        let trampoline_sig = intrinsics.void_ty.fn_type(
            &[
                intrinsics.ctx_ptr_ty.as_basic_type_enum(), // vmctx ptr
                func_type
                    .ptr_type(AddressSpace::Generic)
                    .as_basic_type_enum(), // func ptr
                intrinsics.i64_ptr_ty.as_basic_type_enum(), // args ptr
                intrinsics.i64_ptr_ty.as_basic_type_enum(), // returns ptr
            ],
            false,
        );

        let trampoline_func = module.add_function(
            &format!("trmp{}", sig_index.index()),
            trampoline_sig,
            Some(Linkage::External),
        );

        generate_trampoline(
            trampoline_func,
            sig,
            &func_attrs,
            context,
            builder,
            intrinsics,
        )?;
    }
    Ok(())
}

pub fn type_to_llvm<'ctx>(intrinsics: &Intrinsics<'ctx>, ty: Type) -> BasicTypeEnum<'ctx> {
    match ty {
        Type::I32 => intrinsics.i32_ty.as_basic_type_enum(),
        Type::I64 => intrinsics.i64_ty.as_basic_type_enum(),
        Type::F32 => intrinsics.f32_ty.as_basic_type_enum(),
        Type::F64 => intrinsics.f64_ty.as_basic_type_enum(),
        Type::V128 => intrinsics.i128_ty.as_basic_type_enum(),
    }
}

fn generate_trampoline<'ctx>(
    trampoline_func: FunctionValue,
    func_sig: &FuncSig,
    func_attrs: &Vec<(Attribute, AttributeLoc)>,
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    intrinsics: &Intrinsics<'ctx>,
) -> Result<(), String> {
    let entry_block = context.append_basic_block(trampoline_func, "entry");
    builder.position_at_end(entry_block);

    let (vmctx_ptr, func_ptr, args_ptr, returns_ptr) = match trampoline_func.get_params().as_slice()
    {
        &[vmctx_ptr, func_ptr, args_ptr, returns_ptr] => (
            vmctx_ptr,
            func_ptr.into_pointer_value(),
            args_ptr.into_pointer_value(),
            returns_ptr.into_pointer_value(),
        ),
        _ => return Err("trampoline function unimplemented".to_string()),
    };

    let cast_ptr_ty = |wasmer_ty| match wasmer_ty {
        Type::I32 => intrinsics.i32_ptr_ty,
        Type::F32 => intrinsics.f32_ptr_ty,
        Type::I64 => intrinsics.i64_ptr_ty,
        Type::F64 => intrinsics.f64_ptr_ty,
        Type::V128 => intrinsics.i128_ptr_ty,
    };

    let mut args_vec = Vec::with_capacity(func_sig.params().len() + 1);

    let func_sig_returns_bitwidths = func_sig
        .returns()
        .iter()
        .map(|ty| match ty {
            Type::I32 | Type::F32 => 32,
            Type::I64 | Type::F64 => 64,
            Type::V128 => 128,
        })
        .collect::<Vec<i32>>();

    let _is_sret = match func_sig_returns_bitwidths.as_slice() {
        []
        | [_]
        | [32, 64]
        | [64, 32]
        | [64, 64]
        | [32, 32]
        | [32, 32, 32]
        | [32, 32, 64]
        | [64, 32, 32]
        | [32, 32, 32, 32] => false,
        _ => {
            let basic_types: Vec<_> = func_sig
                .returns()
                .iter()
                .map(|&ty| type_to_llvm(intrinsics, ty))
                .collect();

            let sret_ty = context.struct_type(&basic_types, false);
            args_vec.push(builder.build_alloca(sret_ty, "sret").into());

            true
        }
    };

    args_vec.push(vmctx_ptr);

    let mut i = 0;
    for param_ty in func_sig.params().iter() {
        let index = intrinsics.i32_ty.const_int(i as _, false);
        let item_pointer = unsafe { builder.build_in_bounds_gep(args_ptr, &[index], "arg_ptr") };

        let casted_pointer_type = cast_ptr_ty(*param_ty);

        let typed_item_pointer =
            builder.build_pointer_cast(item_pointer, casted_pointer_type, "typed_arg_pointer");

        let arg = builder.build_load(typed_item_pointer, "arg");
        args_vec.push(arg);
        i = i + 1;
        if *param_ty == Type::V128 {
            i = i + 1;
        }
    }

    let call_site = builder.build_call(func_ptr, &args_vec, "call");

    for (attr, attr_loc) in func_attrs {
        call_site.add_attribute(*attr_loc, *attr);
    }

    let rets = abi::rets_from_call(builder, intrinsics, call_site, func_sig);
    let mut idx = 0;
    rets.iter().for_each(|v| {
        let ptr = unsafe {
            builder.build_gep(returns_ptr, &[intrinsics.i32_ty.const_int(idx, false)], "")
        };
        let ptr = builder.build_pointer_cast(ptr, v.get_type().ptr_type(AddressSpace::Generic), "");
        builder.build_store(ptr, *v);
        if v.get_type() == intrinsics.i128_ty.as_basic_type_enum() {
            idx = idx + 1;
        }
        idx = idx + 1;
    });

    builder.build_return(None);
    Ok(())
}
