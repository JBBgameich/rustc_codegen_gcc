use std::borrow::Cow;
use std::convert::TryFrom;
use std::ops::{Deref, Range};
use std::iter::TrustedLen;

use gccjit::{
    BinaryOp,
    Block,
    ComparisonOp,
    Function,
    LValue,
    RValue,
    ToRValue,
    Type,
    UnaryOp,
};
use rustc_codegen_ssa::MemFlags;
use rustc_codegen_ssa::base::to_immediate;
use rustc_codegen_ssa::common::{AtomicOrdering, AtomicRmwBinOp, IntPredicate, RealPredicate, SynchronizationScope};
use rustc_codegen_ssa::mir::operand::{OperandRef, OperandValue};
use rustc_codegen_ssa::mir::place::PlaceRef;
use rustc_codegen_ssa::traits::{
    BackendTypes,
    BaseTypeMethods,
    BuilderMethods,
    ConstMethods,
    DerivedTypeMethods,
    HasCodegen,
    OverflowOp,
    StaticBuilderMethods,
};
use rustc_middle::ty::{ParamEnv, Ty, TyCtxt};
use rustc_middle::ty::layout::{HasParamEnv, HasTyCtxt, TyAndLayout};
use rustc_span::def_id::DefId;
use rustc_target::abi::{
    self,
    Align,
    HasDataLayout,
    LayoutOf,
    Size,
    TargetDataLayout,
};
use rustc_target::spec::{HasTargetSpec, Target};

use crate::common::{SignType, TypeReflection, type_is_pointer};
use crate::context::CodegenCx;
use crate::type_of::LayoutGccExt;

// TODO
type Funclet = ();

// TODO: remove this variable.
static mut RETURN_VALUE_COUNT: usize = 0;

pub struct Builder<'a: 'gcc, 'gcc, 'tcx> {
    pub cx: &'a CodegenCx<'gcc, 'tcx>,
    pub block: Option<Block<'gcc>>,
}

impl<'gcc, 'tcx> Builder<'_, 'gcc, 'tcx> {
    fn assign(&self, lvalue: LValue<'gcc>, value: RValue<'gcc>) {
        self.llbb().add_assignment(None, lvalue, value);
    }

    fn assign_op(&self, lvalue: LValue<'gcc>, binary_op: BinaryOp, value: RValue<'gcc>) {
        self.block.expect("block").add_assignment_op(None, lvalue, binary_op, value);
    }

    fn check_call<'b>(&mut self, typ: &str, func: Function<'gcc>, args: &'b [RValue<'gcc>]) -> Cow<'b, [RValue<'gcc>]> {
        //let mut fn_ty = self.cx.val_ty(func);
        // Strip off pointers
        /*while self.cx.type_kind(fn_ty) == TypeKind::Pointer {
            fn_ty = self.cx.element_type(fn_ty);
        }*/

        /*assert!(
            self.cx.type_kind(fn_ty) == TypeKind::Function,
            "builder::{} not passed a function, but {:?}",
            typ,
            fn_ty
        );

        let param_tys = self.cx.func_params_types(fn_ty);

        let all_args_match = param_tys
            .iter()
            .zip(args.iter().map(|&v| self.val_ty(v)))
            .all(|(expected_ty, actual_ty)| *expected_ty == actual_ty);*/

        let mut all_args_match = true;
        let mut param_types = vec![];
        let param_count = func.get_param_count();
        for (index, arg) in args.iter().enumerate().take(param_count) {
            let param = func.get_param(index as i32);
            let param = param.to_rvalue().get_type();
            if param != arg.get_type() {
                all_args_match = false;
            }
            param_types.push(param);
        }

        if all_args_match {
            return Cow::Borrowed(args);
        }

        let casted_args: Vec<_> = param_types
            .into_iter()
            .zip(args.iter())
            .enumerate()
            .map(|(i, (expected_ty, &actual_val))| {
                let actual_ty = actual_val.get_type();
                if expected_ty != actual_ty {
                    /*debug!(
                        "type mismatch in function call of {:?}. \
                            Expected {:?} for param {}, got {:?}; injecting bitcast",
                        func, expected_ty, i, actual_ty
                    );*/
                    /*println!(
                        "type mismatch in function call of {:?}. \
                            Expected {:?} for param {}, got {:?}; injecting bitcast",
                        func, expected_ty, i, actual_ty
                    );*/
                    self.bitcast(actual_val, expected_ty)
                }
                else {
                    actual_val
                }
            })
            .collect();

        Cow::Owned(casted_args)
    }

    fn check_ptr_call<'b>(&mut self, typ: &str, func_ptr: RValue<'gcc>, args: &'b [RValue<'gcc>]) -> Cow<'b, [RValue<'gcc>]> {
        //let mut fn_ty = self.cx.val_ty(func);
        // Strip off pointers
        /*while self.cx.type_kind(fn_ty) == TypeKind::Pointer {
            fn_ty = self.cx.element_type(fn_ty);
        }*/

        /*assert!(
            self.cx.type_kind(fn_ty) == TypeKind::Function,
            "builder::{} not passed a function, but {:?}",
            typ,
            fn_ty
        );

        let param_tys = self.cx.func_params_types(fn_ty);

        let all_args_match = param_tys
            .iter()
            .zip(args.iter().map(|&v| self.val_ty(v)))
            .all(|(expected_ty, actual_ty)| *expected_ty == actual_ty);*/

        let mut all_args_match = true;
        let mut param_types = vec![];
        let gcc_func = func_ptr.get_type().is_function_ptr_type().expect("function ptr");
        for (index, arg) in args.iter().enumerate().take(gcc_func.get_param_count()) {
            let param = gcc_func.get_param_type(index);
            if param != arg.get_type() {
                all_args_match = false;
            }
            param_types.push(param);
        }

        if all_args_match {
            return Cow::Borrowed(args);
        }

        let casted_args: Vec<_> = param_types
            .into_iter()
            .zip(args.iter())
            .enumerate()
            .map(|(i, (expected_ty, &actual_val))| {
                let actual_ty = actual_val.get_type();
                if expected_ty != actual_ty {
                    /*debug!(
                        "type mismatch in function call of {:?}. \
                            Expected {:?} for param {}, got {:?}; injecting bitcast",
                        func, expected_ty, i, actual_ty
                    );*/
                    /*println!(
                        "type mismatch in function call of {:?}. \
                            Expected {:?} for param {}, got {:?}; injecting bitcast",
                        func, expected_ty, i, actual_ty
                    );*/
                    self.bitcast(actual_val, expected_ty)
                }
                else {
                    actual_val
                }
            })
            .collect();

        Cow::Owned(casted_args)
    }

    fn check_store(&mut self, val: RValue<'gcc>, ptr: RValue<'gcc>) -> RValue<'gcc> {
        let dest_ptr_ty = self.cx.val_ty(ptr).make_pointer(); // TODO: make sure make_pointer() is okay here.
        let stored_ty = self.cx.val_ty(val);
        let stored_ptr_ty = self.cx.type_ptr_to(stored_ty);

        //assert_eq!(self.cx.type_kind(dest_ptr_ty), TypeKind::Pointer);

        if dest_ptr_ty == stored_ptr_ty {
            ptr
        }
        else {
            /*debug!(
                "type mismatch in store. \
                    Expected {:?}, got {:?}; inserting bitcast",
                dest_ptr_ty, stored_ptr_ty
            );*/
            /*println!(
                "type mismatch in store. \
                    Expected {:?}, got {:?}; inserting bitcast",
                dest_ptr_ty, stored_ptr_ty
            );*/
            //ptr
            self.bitcast(ptr, stored_ptr_ty)
        }
    }

    pub fn current_func(&self) -> Function<'gcc> {
        self.block.expect("block").get_function()
    }

    fn function_call(&mut self, func: RValue<'gcc>, args: &[RValue<'gcc>], funclet: Option<&Funclet>) -> RValue<'gcc> {
        //debug!("call {:?} with args ({:?})", func, args);

        // TODO: remove when the API supports a different type for functions.
        let func: Function<'gcc> = self.cx.rvalue_as_function(func);
        let args = self.check_call("call", func, args);
        //let bundle = funclet.map(|funclet| funclet.bundle());
        //let bundle = bundle.as_ref().map(|b| &*b.raw);

        // gccjit requires to use the result of functions, even when it's not used.
        // That's why we assign the result to a local or call add_eval().
        let return_type = func.get_return_type();
        let current_block = self.current_block.borrow().expect("block");
        let void_type = self.context.new_type::<()>();
        let current_func = current_block.get_function();
        if return_type != void_type {
            unsafe { RETURN_VALUE_COUNT += 1 };
            let result = current_func.new_local(None, return_type, &format!("returnValue{}", unsafe { RETURN_VALUE_COUNT }));
            current_block.add_assignment(None, result, self.cx.context.new_call(None, func, &args));
            result.to_rvalue()
        }
        else {
            current_block.add_eval(None, self.cx.context.new_call(None, func, &args));
            // Return dummy value when not having return value.
            self.context.new_rvalue_from_long(self.isize_type, 0)
        }
    }

    fn function_ptr_call(&mut self, mut func_ptr: RValue<'gcc>, args: &[RValue<'gcc>], funclet: Option<&Funclet>) -> RValue<'gcc> {
        //debug!("func ptr call {:?} with args ({:?})", func, args);

        let args = self.check_ptr_call("call", func_ptr, args);
        //let bundle = funclet.map(|funclet| funclet.bundle());
        //let bundle = bundle.as_ref().map(|b| &*b.raw);

        // gccjit requires to use the result of functions, even when it's not used.
        // That's why we assign the result to a local or call add_eval().
        let gcc_func = func_ptr.get_type().is_function_ptr_type().expect("function ptr");
        let return_type = gcc_func.get_return_type();
        let current_block = self.current_block.borrow().expect("block");
        let void_type = self.context.new_type::<()>();
        let current_func = current_block.get_function();

        if return_type != void_type {
            unsafe { RETURN_VALUE_COUNT += 1 };
            let result = current_func.new_local(None, return_type, &format!("returnValue{}", unsafe { RETURN_VALUE_COUNT }));
            current_block.add_assignment(None, result, self.cx.context.new_call_through_ptr(None, func_ptr, &args));
            result.to_rvalue()
        }
        else {
            current_block.add_eval(None, self.cx.context.new_call_through_ptr(None, func_ptr, &args));
            // Return dummy value when not having return value.
            self.context.new_rvalue_from_long(self.isize_type, 0)
        }
    }

    pub fn overflow_call(&mut self, func: Function<'gcc>, args: &[RValue<'gcc>], funclet: Option<&Funclet>) -> RValue<'gcc> {
        //debug!("overflow_call {:?} with args ({:?})", func, args);

        //let bundle = funclet.map(|funclet| funclet.bundle());
        //let bundle = bundle.as_ref().map(|b| &*b.raw);

        // gccjit requires to use the result of functions, even when it's not used.
        // That's why we assign the result to a local.
        let return_type = self.context.new_type::<bool>();
        let current_block = self.current_block.borrow().expect("block");
        let current_func = current_block.get_function();
        // TODO: return the new_call() directly? Since the overflow function has no side-effects.
        unsafe { RETURN_VALUE_COUNT += 1 };
        let result = current_func.new_local(None, return_type, &format!("returnValue{}", unsafe { RETURN_VALUE_COUNT }));
        current_block.add_assignment(None, result, self.cx.context.new_call(None, func, &args));
        result.to_rvalue()
    }
}

impl<'gcc, 'tcx> HasCodegen<'tcx> for Builder<'_, 'gcc, 'tcx> {
    type CodegenCx = CodegenCx<'gcc, 'tcx>;
}

impl<'tcx> HasTyCtxt<'tcx> for Builder<'_, '_, 'tcx> {
    fn tcx(&self) -> TyCtxt<'tcx> {
        self.cx.tcx()
    }
}

impl HasDataLayout for Builder<'_, '_, '_> {
    fn data_layout(&self) -> &TargetDataLayout {
        self.cx.data_layout()
    }
}

impl<'tcx> LayoutOf for Builder<'_, '_, 'tcx> {
    type Ty = Ty<'tcx>;
    type TyAndLayout = TyAndLayout<'tcx>;

    fn layout_of(&self, ty: Ty<'tcx>) -> Self::TyAndLayout {
        self.cx.layout_of(ty)
    }
}

impl<'gcc, 'tcx> Deref for Builder<'_, 'gcc, 'tcx> {
    type Target = CodegenCx<'gcc, 'tcx>;

    fn deref(&self) -> &Self::Target {
        self.cx
    }
}

impl<'gcc, 'tcx> BackendTypes for Builder<'_, 'gcc, 'tcx> {
    type Value = <CodegenCx<'gcc, 'tcx> as BackendTypes>::Value;
    type Function = <CodegenCx<'gcc, 'tcx> as BackendTypes>::Function;
    type BasicBlock = <CodegenCx<'gcc, 'tcx> as BackendTypes>::BasicBlock;
    type Type = <CodegenCx<'gcc, 'tcx> as BackendTypes>::Type;
    type Funclet = <CodegenCx<'gcc, 'tcx> as BackendTypes>::Funclet;

    type DIScope = <CodegenCx<'gcc, 'tcx> as BackendTypes>::DIScope;
    type DIVariable = <CodegenCx<'gcc, 'tcx> as BackendTypes>::DIVariable;
}

impl<'a, 'gcc, 'tcx> BuilderMethods<'a, 'tcx> for Builder<'a, 'gcc, 'tcx> {
    fn new_block<'b>(cx: &'a CodegenCx<'gcc, 'tcx>, func: RValue<'gcc>, name: &'b str) -> Self {
        let func = cx.rvalue_as_function(func);
        let block = func.new_block(name);
        let mut bx = Builder::with_cx(cx);
        bx.position_at_end(block);
        bx
    }

    fn with_cx(cx: &'a CodegenCx<'gcc, 'tcx>) -> Self {
        Builder {
            cx,
            block: None,
        }
    }

    fn build_sibling_block(&self, name: &str) -> Self {
        let func = self.llbb().get_function();
        // TODO: this is a wrong cast.
        let func: RValue<'gcc> = unsafe { std::mem::transmute(func) };
        Builder::new_block(self.cx, func, name)
    }

    fn llbb(&self) -> Block<'gcc> {
        self.block.expect("block")
    }

    fn position_at_end(&mut self, block: Block<'gcc>) {
        *self.cx.current_block.borrow_mut() = Some(block);
        self.block = Some(block);
    }

    fn ret_void(&mut self) {
        self.llbb().end_with_void_return(None)
    }

    fn ret(&mut self, value: RValue<'gcc>) {
        self.llbb().end_with_return(None, value);
    }

    fn br(&mut self, dest: Block<'gcc>) {
        self.llbb().end_with_jump(None, dest)
    }

    fn cond_br(&mut self, cond: RValue<'gcc>, then_block: Block<'gcc>, else_block: Block<'gcc>) {
        self.llbb().end_with_conditional(None, cond, then_block, else_block)
    }

    fn switch(&mut self, value: RValue<'gcc>, default_block: Block<'gcc>, cases: impl ExactSizeIterator<Item = (u128, Block<'gcc>)> + TrustedLen) {
        let mut gcc_cases = vec![];
        let typ = self.val_ty(value);
        for (on_val, dest) in cases {
            let on_val = self.const_uint_big(typ, on_val);
            gcc_cases.push(self.context.new_case(on_val, on_val, dest));
        }
        self.block.expect("block").end_with_switch(None, value, default_block, &gcc_cases);
    }

    fn invoke(&mut self, func: RValue<'gcc>, args: &[RValue<'gcc>], then: Block<'gcc>, catch: Block<'gcc>, funclet: Option<&Funclet>) -> RValue<'gcc> {
        unimplemented!();
        /*debug!("invoke {:?} with args ({:?})", func, args);

        let args = self.check_call("invoke", func, args);
        let bundle = funclet.map(|funclet| funclet.bundle());
        let bundle = bundle.as_ref().map(|b| &*b.raw);

        unsafe {
            llvm::LLVMRustBuildInvoke(
                self.llbuilder,
                func,
                args.as_ptr(),
                args.len() as c_uint,
                then,
                catch,
                bundle,
                UNNAMED,
            )
        }*/
    }

    fn unreachable(&mut self) {
        let func = self.context.get_builtin_function("__builtin_unreachable");
        let block = self.block.expect("block");
        block.add_eval(None, self.context.new_call(None, func, &[]));
        let return_type = block.get_function().get_return_type();
        let void_type = self.context.new_type::<()>();
        if return_type == void_type {
            block.end_with_void_return(None)
        }
        else {
            let return_value = self.current_func()
                .new_local(None, return_type, "unreachableReturn");
            block.end_with_return(None, return_value)
        }
    }

    fn add(&mut self, a: RValue<'gcc>, b: RValue<'gcc>) -> RValue<'gcc> {
        a + b
    }

    fn fadd(&mut self, a: RValue<'gcc>, b: RValue<'gcc>) -> RValue<'gcc> {
        a + b
    }

    fn sub(&mut self, a: RValue<'gcc>, b: RValue<'gcc>) -> RValue<'gcc> {
        a - b
    }

    fn fsub(&mut self, a: RValue<'gcc>, b: RValue<'gcc>) -> RValue<'gcc> {
        a - b
    }

    fn mul(&mut self, a: RValue<'gcc>, b: RValue<'gcc>) -> RValue<'gcc> {
        a * b
    }

    fn fmul(&mut self, a: RValue<'gcc>, b: RValue<'gcc>) -> RValue<'gcc> {
        a * b
    }

    fn udiv(&mut self, a: RValue<'gcc>, b: RValue<'gcc>) -> RValue<'gcc> {
        a / b
    }

    fn exactudiv(&mut self, a: RValue<'gcc>, b: RValue<'gcc>) -> RValue<'gcc> {
        // TODO: poison if not exact.
        a / b
    }

    fn sdiv(&mut self, a: RValue<'gcc>, b: RValue<'gcc>) -> RValue<'gcc> {
        a / b
    }

    fn exactsdiv(&mut self, a: RValue<'gcc>, b: RValue<'gcc>) -> RValue<'gcc> {
        // TODO: posion if not exact.
        a / b
    }

    fn fdiv(&mut self, a: RValue<'gcc>, b: RValue<'gcc>) -> RValue<'gcc> {
        a / b
    }

    fn urem(&mut self, a: RValue<'gcc>, b: RValue<'gcc>) -> RValue<'gcc> {
        a % b
    }

    fn srem(&mut self, a: RValue<'gcc>, b: RValue<'gcc>) -> RValue<'gcc> {
        a % b
    }

    fn frem(&mut self, a: RValue<'gcc>, b: RValue<'gcc>) -> RValue<'gcc> {
        a % b
    }

    fn shl(&mut self, a: RValue<'gcc>, b: RValue<'gcc>) -> RValue<'gcc> {
        // FIXME: remove the casts when libgccjit can shift an unsigned number by an unsigned number.
        let a_type = a.get_type();
        let b_type = b.get_type();
        if a_type.is_unsigned(self) && b_type.is_signed(self) {
            //println!("shl: {:?} -> {:?}", a, b_type);
            let a = self.context.new_cast(None, a, b_type);
            let result = a << b;
            //println!("shl: {:?} -> {:?}", result, a_type);
            self.context.new_cast(None, result, a_type)
        }
        else if a_type.is_signed(self) && b_type.is_unsigned(self) {
            //println!("shl: {:?} -> {:?}", b, a_type);
            let b = self.context.new_cast(None, b, a_type);
            a << b
        }
        else {
            a << b
        }
    }

    fn lshr(&mut self, a: RValue<'gcc>, b: RValue<'gcc>) -> RValue<'gcc> {
        // FIXME: remove the casts when libgccjit can shift an unsigned number by an unsigned number.
        // TODO: cast to unsigned to do a logical shift if that does not work.
        let a_type = a.get_type();
        let b_type = b.get_type();
        if a_type.is_unsigned(self) && b_type.is_signed(self) {
            //println!("lshl: {:?} -> {:?}", a, b_type);
            let a = self.context.new_cast(None, a, b_type);
            let result = a >> b;
            //println!("lshl: {:?} -> {:?}", result, a_type);
            self.context.new_cast(None, result, a_type)
        }
        else if a_type.is_signed(self) && b_type.is_unsigned(self) {
            //println!("lshl: {:?} -> {:?}", b, a_type);
            let b = self.context.new_cast(None, b, a_type);
            a >> b
        }
        else {
            a >> b
        }
    }

    fn ashr(&mut self, a: RValue<'gcc>, b: RValue<'gcc>) -> RValue<'gcc> {
        // TODO: check whether behavior is an arithmetic shift for >> .
        // FIXME: remove the casts when libgccjit can shift an unsigned number by an unsigned number.
        let a_type = a.get_type();
        let b_type = b.get_type();
        if a_type.is_unsigned(self) && b_type.is_signed(self) {
            //println!("ashl: {:?} -> {:?}", a, b_type);
            let a = self.context.new_cast(None, a, b_type);
            let result = a >> b;
            //println!("ashl: {:?} -> {:?}", result, a_type);
            self.context.new_cast(None, result, a_type)
        }
        else if a_type.is_signed(self) && b_type.is_unsigned(self) {
            //println!("ashl: {:?} -> {:?}", b, a_type);
            let b = self.context.new_cast(None, b, a_type);
            a >> b
        }
        else {
            a >> b
        }
    }

    fn and(&mut self, a: RValue<'gcc>, b: RValue<'gcc>) -> RValue<'gcc> {
        // FIXME: hack by putting the result in a variable to workaround this bug:
        // https://gcc.gnu.org/bugzilla//show_bug.cgi?id=95498
        let res = self.current_func().new_local(None, b.get_type(), "andResult");
        self.llbb().add_assignment(None, res, a & b);
        res.to_rvalue()
    }

    fn or(&mut self, a: RValue<'gcc>, b: RValue<'gcc>) -> RValue<'gcc> {
        // FIXME: hack by putting the result in a variable to workaround this bug:
        // https://gcc.gnu.org/bugzilla//show_bug.cgi?id=95498
        let res = self.current_func().new_local(None, b.get_type(), "orResult");
        self.llbb().add_assignment(None, res, a | b);
        res.to_rvalue()
    }

    fn xor(&mut self, a: RValue<'gcc>, b: RValue<'gcc>) -> RValue<'gcc> {
        a ^ b
    }

    fn neg(&mut self, a: RValue<'gcc>) -> RValue<'gcc> {
        // TODO: use new_unary_op()?
        self.cx.context.new_rvalue_from_long(a.get_type(), 0) - a
    }

    fn fneg(&mut self, a: RValue<'gcc>) -> RValue<'gcc> {
        // TODO: use new_unary_op()?
        self.cx.context.new_rvalue_from_long(a.get_type(), 0) - a
    }

    fn not(&mut self, a: RValue<'gcc>) -> RValue<'gcc> {
        let operation =
            if a.get_type().is_bool() {
                UnaryOp::LogicalNegate
            }
            else {
                UnaryOp::BitwiseNegate
            };
        self.cx.context.new_unary_op(None, operation, a.get_type(), a)
    }

    fn unchecked_sadd(&mut self, a: RValue<'gcc>, b: RValue<'gcc>) -> RValue<'gcc> {
        a + b
    }

    fn unchecked_uadd(&mut self, a: RValue<'gcc>, b: RValue<'gcc>) -> RValue<'gcc> {
        a + b
    }

    fn unchecked_ssub(&mut self, a: RValue<'gcc>, b: RValue<'gcc>) -> RValue<'gcc> {
        a - b
    }

    fn unchecked_usub(&mut self, a: RValue<'gcc>, b: RValue<'gcc>) -> RValue<'gcc> {
        // TODO: should generate poison value?
        a - b
    }

    fn unchecked_smul(&mut self, a: RValue<'gcc>, b: RValue<'gcc>) -> RValue<'gcc> {
        a * b
    }

    fn unchecked_umul(&mut self, a: RValue<'gcc>, b: RValue<'gcc>) -> RValue<'gcc> {
        a * b
    }

    fn fadd_fast(&mut self, lhs: RValue<'gcc>, rhs: RValue<'gcc>) -> RValue<'gcc> {
        unimplemented!();
        /*unsafe {
            let instr = llvm::LLVMBuildFAdd(self.llbuilder, lhs, rhs, UNNAMED);
            llvm::LLVMRustSetHasUnsafeAlgebra(instr);
            instr
        }*/
    }

    fn fsub_fast(&mut self, lhs: RValue<'gcc>, rhs: RValue<'gcc>) -> RValue<'gcc> {
        unimplemented!();
        /*unsafe {
            let instr = llvm::LLVMBuildFSub(self.llbuilder, lhs, rhs, UNNAMED);
            llvm::LLVMRustSetHasUnsafeAlgebra(instr);
            instr
        }*/
    }

    fn fmul_fast(&mut self, lhs: RValue<'gcc>, rhs: RValue<'gcc>) -> RValue<'gcc> {
        unimplemented!();
        /*unsafe {
            let instr = llvm::LLVMBuildFMul(self.llbuilder, lhs, rhs, UNNAMED);
            llvm::LLVMRustSetHasUnsafeAlgebra(instr);
            instr
        }*/
    }

    fn fdiv_fast(&mut self, lhs: RValue<'gcc>, rhs: RValue<'gcc>) -> RValue<'gcc> {
        unimplemented!();
        /*unsafe {
            let instr = llvm::LLVMBuildFDiv(self.llbuilder, lhs, rhs, UNNAMED);
            llvm::LLVMRustSetHasUnsafeAlgebra(instr);
            instr
        }*/
    }

    fn frem_fast(&mut self, lhs: RValue<'gcc>, rhs: RValue<'gcc>) -> RValue<'gcc> {
        unimplemented!();
        /*unsafe {
            let instr = llvm::LLVMBuildFRem(self.llbuilder, lhs, rhs, UNNAMED);
            llvm::LLVMRustSetHasUnsafeAlgebra(instr);
            instr
        }*/
    }

    fn checked_binop(&mut self, oop: OverflowOp, typ: Ty<'_>, lhs: Self::Value, rhs: Self::Value) -> (Self::Value, Self::Value) {
        use rustc_ast::ast::IntTy::*;
        use rustc_ast::ast::UintTy::*;
        use rustc_middle::ty::{Int, Uint};

        let new_kind =
            match typ.kind {
                Int(t @ Isize) => Int(t.normalize(self.tcx.sess.target.ptr_width)),
                Uint(t @ Usize) => Uint(t.normalize(self.tcx.sess.target.ptr_width)),
                ref t @ (Uint(_) | Int(_)) => t.clone(),
                _ => panic!("tried to get overflow intrinsic for op applied to non-int type"),
            };

        // TODO: remove duplication with intrinsic?
        let name =
            match oop {
                OverflowOp::Add =>
                    match new_kind {
                        Int(I8) => "__builtin_add_overflow",
                        Int(I16) => "__builtin_add_overflow",
                        Int(I32) => "__builtin_sadd_overflow",
                        Int(I64) => "__builtin_saddll_overflow",
                        Int(I128) => "__builtin_saddll_overflow",

                        Uint(U8) => "__builtin_add_overflow",
                        Uint(U16) => "__builtin_add_overflow",
                        Uint(U32) => "__builtin_uadd_overflow",
                        Uint(U64) => "__builtin_uaddll_overflow",
                        Uint(U128) => "__builtin_uaddll_overflow",

                        _ => unreachable!(),
                    },
                OverflowOp::Sub =>
                    match new_kind {
                        Int(I8) => "__builtin_sub_overflow",
                        Int(I16) => "__builtin_sub_overflow",
                        Int(I32) => "__builtin_ssub_overflow",
                        Int(I64) => "__builtin_ssubll_overflow",
                        Int(I128) => "__builtin_ssubll_overflow",

                        Uint(U8) => "__builtin_sub_overflow",
                        Uint(U16) => "__builtin_sub_overflow",
                        Uint(U32) => "__builtin_usub_overflow",
                        Uint(U64) => "__builtin_usubll_overflow",
                        Uint(U128) => "__builtin_usubll_overflow",

                        _ => unreachable!(),
                    },
                OverflowOp::Mul =>
                    match new_kind {
                        Int(I8) => "__builtin_mul_overflow",
                        Int(I16) => "__builtin_mul_overflow",
                        Int(I32) => "__builtin_smul_overflow",
                        Int(I64) => "__builtin_smulll_overflow",
                        Int(I128) => "__builtin_smulll_overflow",

                        Uint(U8) => "__builtin_mul_overflow",
                        Uint(U16) => "__builtin_mul_overflow",
                        Uint(U32) => "__builtin_umul_overflow",
                        Uint(U64) => "__builtin_umulll_overflow",
                        Uint(U128) => "__builtin_umulll_overflow",

                        _ => unreachable!(),
                    },
            };

        let intrinsic = self.context.get_builtin_function(&name);
        let res = self.current_func()
            // TODO: is it correct to use rhs type instead of the parameter typ?
            .new_local(None, rhs.get_type(), "binopResult")
            .get_address(None);
        let overflow = self.overflow_call(intrinsic, &[lhs, rhs, res], None);
        (res.dereference(None).to_rvalue(), overflow)
    }

    fn alloca(&mut self, ty: Type<'gcc>, align: Align) -> RValue<'gcc> {
        let aligned_type = ty.get_aligned(align.bytes());
        // TODO: It might be better to return a LValue, but fixing the rustc API is non-trivial.
        self.current_func().new_local(None, aligned_type, "stack_var").get_address(None)
    }

    fn dynamic_alloca(&mut self, ty: Type<'gcc>, align: Align) -> RValue<'gcc> {
        unimplemented!();
        /*unsafe {
            let alloca = llvm::LLVMBuildAlloca(self.llbuilder, ty, UNNAMED);
            llvm::LLVMSetAlignment(alloca, align.bytes() as c_uint);
            alloca
        }*/
    }

    fn array_alloca(&mut self, ty: Type<'gcc>, len: RValue<'gcc>, align: Align) -> RValue<'gcc> {
        unimplemented!();
        /*unsafe {
            let alloca = llvm::LLVMBuildArrayAlloca(self.llbuilder, ty, len, UNNAMED);
            llvm::LLVMSetAlignment(alloca, align.bytes() as c_uint);
            alloca
        }*/
    }

    fn load(&mut self, ptr: RValue<'gcc>, align: Align) -> RValue<'gcc> {
        let block = self.llbb();
        let function = block.get_function();
        // NOTE: instead of returning the dereference here, we have to assign it to a variable in
        // the current basic block. Otherwise, it could be used in another basic block, causing a
        // dereference after a drop, for instance.
        // TODO: handle align.
        let deref = ptr.dereference(None).to_rvalue();
        let value_type = deref.get_type();
        unsafe { RETURN_VALUE_COUNT += 1 };
        let loaded_value = function.new_local(None, value_type, &format!("loadedValue{}", unsafe { RETURN_VALUE_COUNT }));
        block.add_assignment(None, loaded_value, deref);
        loaded_value.to_rvalue()
    }

    fn volatile_load(&mut self, ptr: RValue<'gcc>) -> RValue<'gcc> {
        //println!("5: volatile load: {:?} to {:?}", ptr, ptr.get_type().make_volatile());
        let ptr = self.context.new_cast(None, ptr, ptr.get_type().make_volatile());
        //println!("6");
        ptr.dereference(None).to_rvalue()
    }

    fn atomic_load(&mut self, ptr: RValue<'gcc>, order: AtomicOrdering, size: Size) -> RValue<'gcc> {
        ptr.dereference(None).to_rvalue()
        // TODO: replace with following code when libgccjit supports __atomic_load types.
        // TODO: handle alignment.
        /*let atomic_load = self.context.get_builtin_function("__atomic_load");
        let ordering = self.context.new_rvalue_from_int(self.i32_type, order.to_gcc());
        let block = self.block.expect("block");
        let result_type = ptr.dereference(None).to_rvalue().get_type();
        let result = block.get_function().new_local(None, result_type, "atomic_load_result");
        block.add_eval(None, self.context.new_call(None, atomic_load, &[ptr, result.get_address(None), ordering]));
        result.to_rvalue()*/
    }

    fn load_operand(&mut self, place: PlaceRef<'tcx, RValue<'gcc>>) -> OperandRef<'tcx, RValue<'gcc>> {
        //debug!("PlaceRef::load: {:?}", place);

        assert_eq!(place.llextra.is_some(), place.layout.is_unsized());

        if place.layout.is_zst() {
            return OperandRef::new_zst(self, place.layout);
        }

        fn scalar_load_metadata<'a, 'gcc, 'tcx>(bx: &mut Builder<'a, 'gcc, 'tcx>, load: RValue<'gcc>, scalar: &abi::Scalar) {
            let vr = scalar.valid_range.clone();
            match scalar.value {
                abi::Int(..) => {
                    let range = scalar.valid_range_exclusive(bx);
                    if range.start != range.end {
                        bx.range_metadata(load, range);
                    }
                }
                abi::Pointer if vr.start() < vr.end() && !vr.contains(&0) => {
                    bx.nonnull_metadata(load);
                }
                _ => {}
            }
        }

        let val =
            if let Some(llextra) = place.llextra {
                OperandValue::Ref(place.llval, Some(llextra), place.align)
            }
            else if place.layout.is_gcc_immediate() {
                let mut const_llval = None;
                /*unsafe {
                    if let Some(global) = llvm::LLVMIsAGlobalVariable(place.llval) {
                        if llvm::LLVMIsGlobalConstant(global) == llvm::True {
                            const_llval = llvm::LLVMGetInitializer(global);
                        }
                    }
                }*/
                let llval = const_llval.unwrap_or_else(|| {
                    let load = self.load(place.llval, place.align);
                    if let abi::Abi::Scalar(ref scalar) = place.layout.abi {
                        scalar_load_metadata(self, load, scalar);
                    }
                    load
                });
                OperandValue::Immediate(to_immediate(self, llval, place.layout))
            }
            else if let abi::Abi::ScalarPair(ref a, ref b) = place.layout.abi {
                let b_offset = a.value.size(self).align_to(b.value.align(self).abi);

                let mut load = |i, scalar: &abi::Scalar, align| {
                    let llptr = self.struct_gep(place.llval, i as u64);
                    let load = self.load(llptr, align);
                    scalar_load_metadata(self, load, scalar);
                    if scalar.is_bool() { self.trunc(load, self.type_i1()) } else { load }
                };

                OperandValue::Pair(
                    load(0, a, place.align),
                    load(1, b, place.align.restrict_for_offset(b_offset)),
                )
            }
            else {
                OperandValue::Ref(place.llval, None, place.align)
            };

        OperandRef { val, layout: place.layout }
    }

    fn write_operand_repeatedly(mut self, cg_elem: OperandRef<'tcx, RValue<'gcc>>, count: u64, dest: PlaceRef<'tcx, RValue<'gcc>>) -> Self {
        let zero = self.const_usize(0);
        let count = self.const_usize(count);
        let start = dest.project_index(&mut self, zero).llval;
        let end = dest.project_index(&mut self, count).llval;

        let mut header_bx = self.build_sibling_block("repeat_loop_header");
        let mut body_bx = self.build_sibling_block("repeat_loop_body");
        let next_bx = self.build_sibling_block("repeat_loop_next");

        let ptr_type = start.get_type();
        let current = self.llbb().get_function().new_local(None, ptr_type, "loop_var");
        let current_val = current.to_rvalue();
        self.assign(current, start);

        self.br(header_bx.llbb());

        let keep_going = header_bx.icmp(IntPredicate::IntNE, current_val, end);
        header_bx.cond_br(keep_going, body_bx.llbb(), next_bx.llbb());

        let align = dest.align.restrict_for_offset(dest.layout.field(self.cx(), 0).size);
        cg_elem.val.store(&mut body_bx, PlaceRef::new_sized_aligned(current_val, cg_elem.layout, align));

        let ptr_value = self.const_usize(1);
        let current_int = body_bx.ptrtoint(current_val, self.usize_type);
        let next = current_int + ptr_value;
        let next = body_bx.inttoptr(next, ptr_type);
        body_bx.llbb().add_assignment(None, current, next);
        body_bx.br(header_bx.llbb());

        next_bx
    }

    fn range_metadata(&mut self, load: RValue<'gcc>, range: Range<u128>) {
        // TODO
        /*if self.sess().target.target.arch == "amdgpu" {
            // amdgpu/LLVM does something weird and thinks a i64 value is
            // split into a v2i32, halving the bitwidth LLVM expects,
            // tripping an assertion. So, for now, just disable this
            // optimization.
            return;
        }

        unsafe {
            let llty = self.cx.val_ty(load);
            let v = [
                self.cx.const_uint_big(llty, range.start),
                self.cx.const_uint_big(llty, range.end),
            ];

            llvm::LLVMSetMetadata(
                load,
                llvm::MD_range as c_uint,
                llvm::LLVMMDNodeInContext(self.cx.llcx, v.as_ptr(), v.len() as c_uint),
            );
        }*/
    }

    fn nonnull_metadata(&mut self, load: RValue<'gcc>) {
        // TODO
        /*unsafe {
            llvm::LLVMSetMetadata(
                load,
                llvm::MD_nonnull as c_uint,
                llvm::LLVMMDNodeInContext(self.cx.llcx, ptr::null(), 0),
            );
        }*/
    }

    fn store(&mut self, val: RValue<'gcc>, ptr: RValue<'gcc>, align: Align) -> RValue<'gcc> {
        self.store_with_flags(val, ptr, align, MemFlags::empty())
    }

    fn store_with_flags(&mut self, val: RValue<'gcc>, ptr: RValue<'gcc>, align: Align, flags: MemFlags) -> RValue<'gcc> {
        //debug!("Store {:?} -> {:?} ({:?})", val, ptr, flags);
        let ptr = self.check_store(val, ptr);
        self.llbb().add_assignment(None, ptr.dereference(None), val);
        /*let align =
            if flags.contains(MemFlags::UNALIGNED) { 1 } else { align.bytes() as c_uint };
        llvm::LLVMSetAlignment(store, align);
        if flags.contains(MemFlags::VOLATILE) {
            llvm::LLVMSetVolatile(store, llvm::True);
        }
        if flags.contains(MemFlags::NONTEMPORAL) {
            // According to LLVM [1] building a nontemporal store must
            // *always* point to a metadata value of the integer 1.
            //
            // [1]: http://llvm.org/docs/LangRef.html#store-instruction
            let one = self.cx.const_i32(1);
            let node = llvm::LLVMMDNodeInContext(self.cx.llcx, &one, 1);
            llvm::LLVMSetMetadata(store, llvm::MD_nontemporal as c_uint, node);
        }*/
        // NOTE: dummy value here since it's never used. FIXME: API should not return a value here?
        self.cx.context.new_rvalue_zero(self.type_i32())
    }

    fn atomic_store(&mut self, value: RValue<'gcc>, ptr: RValue<'gcc>, order: AtomicOrdering, size: Size) {
        self.llbb().add_assignment(None, ptr.dereference(None), value);
        // TODO: replace with the following when atomic builtins are supported.
        // TODO: handle alignment.
        /*let atomic_store = self.context.get_builtin_function("__atomic_store_n");
        let ordering = self.context.new_rvalue_from_int(self.i32_type, order.to_gcc());
        self.llbb()
            .add_eval(None, self.context.new_call(None, atomic_store, &[ptr, value, ordering]));*/
    }

    fn gep(&mut self, ptr: RValue<'gcc>, indices: &[RValue<'gcc>]) -> RValue<'gcc> {
        let mut result = ptr;
        for index in indices {
            result = self.context.new_array_access(None, result, *index).get_address(None).to_rvalue();
        }
        result
    }

    fn inbounds_gep(&mut self, ptr: RValue<'gcc>, indices: &[RValue<'gcc>]) -> RValue<'gcc> {
        // FIXME: would be safer if doing the same thing (loop) as gep.
        // TODO: specify inbounds somehow.
        match indices.len() {
            1 => {
                self.context.new_array_access(None, ptr, indices[0]).get_address(None)
            },
            2 => {
                let array = ptr.dereference(None); // TODO: assert that first index is 0?
                self.context.new_array_access(None, array, indices[1]).get_address(None)
            },
            _ => unimplemented!(),
        }
    }

    fn struct_gep(&mut self, ptr: RValue<'gcc>, idx: u64) -> RValue<'gcc> {
        // FIXME: it would be better if the API only called this on struct, not on arrays.
        assert_eq!(idx as usize as u64, idx);
        let value = ptr.dereference(None).to_rvalue();
        let value_type = value.get_type();

        if value_type.is_array() {
            let index = self.context.new_rvalue_from_long(self.u64_type, i64::try_from(idx).expect("i64::try_from"));
            let element = self.context.new_array_access(None, value, index);
            element.get_address(None)
        }
        else if let Some(vector_type) = value_type.is_vector() {
            let count = vector_type.get_num_units();
            let element_type = vector_type.get_element_type();
            let indexes = vec![self.context.new_rvalue_from_long(element_type, i64::try_from(idx).expect("i64::try_from")); count as usize];
            let indexes = self.context.new_rvalue_from_vector(None, value_type, &indexes);
            let variable = self.current_func.borrow().expect("func")
                .new_local(None, value_type, "vectorVar");
            self.current_block.borrow().expect("block")
                .add_assignment(None, variable, value + indexes);
            variable.get_address(None)
        }
        else if let Some(struct_type) = value_type.is_struct() {
            ptr.dereference_field(None, struct_type.get_field(idx as i32)).get_address(None)
        }
        else {
            panic!("Unexpected type {:?}", value_type);
        }
    }

    /* Casts */
    fn trunc(&mut self, value: RValue<'gcc>, dest_ty: Type<'gcc>) -> RValue<'gcc> {
        // TODO: check that it indeed truncate the value.
        //println!("trunc: {:?} -> {:?}", value, dest_ty);
        self.context.new_cast(None, value, dest_ty)
    }

    fn sext(&mut self, val: RValue<'gcc>, dest_ty: Type<'gcc>) -> RValue<'gcc> {
        unimplemented!();
        //unsafe { llvm::LLVMBuildSExt(self.llbuilder, val, dest_ty, UNNAMED) }
    }

    fn fptoui(&mut self, value: RValue<'gcc>, dest_ty: Type<'gcc>) -> RValue<'gcc> {
        //println!("7: fptoui: {:?} to {:?}", value, dest_ty);
        let ret = self.context.new_cast(None, value, dest_ty);
        //println!("8");
        ret
        //unsafe { llvm::LLVMBuildFPToUI(self.llbuilder, val, dest_ty, UNNAMED) }
    }

    fn fptosi(&mut self, value: RValue<'gcc>, dest_ty: Type<'gcc>) -> RValue<'gcc> {
        self.context.new_cast(None, value, dest_ty)
    }

    fn uitofp(&mut self, value: RValue<'gcc>, dest_ty: Type<'gcc>) -> RValue<'gcc> {
        //println!("1: uitofp: {:?} -> {:?}", value, dest_ty);
        let ret = self.context.new_cast(None, value, dest_ty);
        //println!("2");
        ret
    }

    fn sitofp(&mut self, value: RValue<'gcc>, dest_ty: Type<'gcc>) -> RValue<'gcc> {
        //println!("3: sitofp: {:?} -> {:?}", value, dest_ty);
        let ret = self.context.new_cast(None, value, dest_ty);
        //println!("4");
        ret
    }

    fn fptrunc(&mut self, value: RValue<'gcc>, dest_ty: Type<'gcc>) -> RValue<'gcc> {
        // TODO: make sure it trancates.
        self.context.new_cast(None, value, dest_ty)
    }

    fn fpext(&mut self, value: RValue<'gcc>, dest_ty: Type<'gcc>) -> RValue<'gcc> {
        self.context.new_cast(None, value, dest_ty)
    }

    fn ptrtoint(&mut self, value: RValue<'gcc>, dest_ty: Type<'gcc>) -> RValue<'gcc> {
        self.cx.ptrtoint(self.block.expect("block"), value, dest_ty)
    }

    fn inttoptr(&mut self, value: RValue<'gcc>, dest_ty: Type<'gcc>) -> RValue<'gcc> {
        self.cx.inttoptr(self.block.expect("block"), value, dest_ty)
    }

    fn bitcast(&mut self, value: RValue<'gcc>, dest_ty: Type<'gcc>) -> RValue<'gcc> {
        self.cx.const_bitcast(value, dest_ty)
    }

    fn intcast(&mut self, value: RValue<'gcc>, dest_typ: Type<'gcc>, is_signed: bool) -> RValue<'gcc> {
        // NOTE: is_signed is for value, not dest_typ.
        //println!("intcast: {:?} ({:?}) -> {:?}", value, value.get_type(), dest_typ);
        self.cx.context.new_cast(None, value, dest_typ)
    }

    fn pointercast(&mut self, value: RValue<'gcc>, dest_ty: Type<'gcc>) -> RValue<'gcc> {
        //println!("pointercast: {:?} ({:?}) -> {:?}", value, value.get_type(), dest_ty);
        let val_type = value.get_type();
        match (type_is_pointer(val_type), type_is_pointer(dest_ty)) {
            (false, true) => {
                // NOTE: Projecting a field of a pointer type will attemp a cast from a signed char to
                // a pointer, which is not supported by gccjit.
                return self.cx.context.new_cast(None, self.inttoptr(value, val_type.make_pointer()), dest_ty);
            },
            (false, false) => {
                // When they are not pointers, we want a transmute (or reinterpret_cast).
                //self.cx.context.new_cast(None, value, dest_ty)
                self.bitcast(value, dest_ty)
            },
            (true, true) => self.cx.context.new_cast(None, value, dest_ty),
            (true, false) => unimplemented!(),
        }
    }

    /* Comparisons */
    fn icmp(&mut self, op: IntPredicate, lhs: RValue<'gcc>, rhs: RValue<'gcc>) -> RValue<'gcc> {
        self.context.new_comparison(None, op.to_gcc_comparison(), lhs, rhs)
    }

    fn fcmp(&mut self, op: RealPredicate, lhs: RValue<'gcc>, rhs: RValue<'gcc>) -> RValue<'gcc> {
        self.context.new_comparison(None, op.to_gcc_comparison(), lhs, rhs)
    }

    /* Miscellaneous instructions */
    fn memcpy(&mut self, dst: RValue<'gcc>, dst_align: Align, src: RValue<'gcc>, src_align: Align, size: RValue<'gcc>, flags: MemFlags) {
        if flags.contains(MemFlags::NONTEMPORAL) {
            // HACK(nox): This is inefficient but there is no nontemporal memcpy.
            let val = self.load(src, src_align);
            let ptr = self.pointercast(dst, self.type_ptr_to(self.val_ty(val)));
            self.store_with_flags(val, ptr, dst_align, flags);
            return;
        }
        let size = self.intcast(size, self.type_size_t(), false);
        let is_volatile = flags.contains(MemFlags::VOLATILE);
        let dst = self.pointercast(dst, self.type_i8p());
        let src = self.pointercast(src, self.type_ptr_to(self.type_void()));
        let memcpy = self.context.get_builtin_function("memcpy");
        let block = self.block.expect("block");
        // TODO: handle aligns and is_volatile.
        block.add_eval(None, self.context.new_call(None, memcpy, &[dst, src, size]));
    }

    fn memmove(&mut self, dst: RValue<'gcc>, dst_align: Align, src: RValue<'gcc>, src_align: Align, size: RValue<'gcc>, flags: MemFlags) {
        if flags.contains(MemFlags::NONTEMPORAL) {
            // HACK(nox): This is inefficient but there is no nontemporal memmove.
            let val = self.load(src, src_align);
            let ptr = self.pointercast(dst, self.type_ptr_to(self.val_ty(val)));
            self.store_with_flags(val, ptr, dst_align, flags);
            return;
        }
        let size = self.intcast(size, self.type_size_t(), false);
        let is_volatile = flags.contains(MemFlags::VOLATILE);
        let dst = self.pointercast(dst, self.type_i8p());
        let src = self.pointercast(src, self.type_ptr_to(self.type_void()));

        let memmove = self.context.get_builtin_function("memmove");
        let block = self.block.expect("block");
        // TODO: handle is_volatile.
        block.add_eval(None, self.context.new_call(None, memmove, &[dst, src, size]));
    }

    fn memset(&mut self, ptr: RValue<'gcc>, fill_byte: RValue<'gcc>, size: RValue<'gcc>, align: Align, flags: MemFlags) {
        let is_volatile = flags.contains(MemFlags::VOLATILE);
        let ptr = self.pointercast(ptr, self.type_i8p());
        let memset = self.context.get_builtin_function("memset");
        let block = self.block.expect("block");
        // TODO: handle aligns and is_volatile.
        //println!("memset: {:?} -> {:?}", fill_byte, self.i32_type);
        let fill_byte = self.context.new_cast(None, fill_byte, self.i32_type);
        let size = self.intcast(size, self.type_size_t(), false);
        block.add_eval(None, self.context.new_call(None, memset, &[ptr, fill_byte, size]));
    }

    fn select(&mut self, cond: RValue<'gcc>, then_val: RValue<'gcc>, else_val: RValue<'gcc>) -> RValue<'gcc> {
        let func = self.current_func();
        let variable = func.new_local(None, then_val.get_type(), "selectVar");
        let then_block = func.new_block("then");
        let else_block = func.new_block("else");
        let after_block = func.new_block("after");
        self.llbb().end_with_conditional(None, cond, then_block, else_block);

        then_block.add_assignment(None, variable, then_val);
        then_block.end_with_jump(None, after_block);

        else_block.add_assignment(None, variable, else_val);
        else_block.end_with_jump(None, after_block);

        // NOTE: since jumps were added in a place rustc does not expect, the current blocks in the
        // state need to be updated.
        self.block = Some(after_block);
        *self.cx.current_block.borrow_mut() = Some(after_block);

        variable.to_rvalue()
    }

    #[allow(dead_code)]
    fn va_arg(&mut self, list: RValue<'gcc>, ty: Type<'gcc>) -> RValue<'gcc> {
        unimplemented!();
        //unsafe { llvm::LLVMBuildVAArg(self.llbuilder, list, ty, UNNAMED) }
    }

    fn extract_element(&mut self, vec: RValue<'gcc>, idx: RValue<'gcc>) -> RValue<'gcc> {
        unimplemented!();
        //unsafe { llvm::LLVMBuildExtractElement(self.llbuilder, vec, idx, UNNAMED) }
    }

    fn vector_splat(&mut self, num_elts: usize, elt: RValue<'gcc>) -> RValue<'gcc> {
        unimplemented!();
        /*unsafe {
            let elt_ty = self.cx.val_ty(elt);
            let undef = llvm::LLVMGetUndef(self.type_vector(elt_ty, num_elts as u64));
            let vec = self.insert_element(undef, elt, self.cx.const_i32(0));
            let vec_i32_ty = self.type_vector(self.type_i32(), num_elts as u64);
            self.shuffle_vector(vec, undef, self.const_null(vec_i32_ty))
        }*/
    }

    fn extract_value(&mut self, aggregate_value: RValue<'gcc>, idx: u64) -> RValue<'gcc> {
        // FIXME: it would be better if the API only called this on struct, not on arrays.
        assert_eq!(idx as usize as u64, idx);
        let value_type = aggregate_value.get_type();

        if value_type.is_array() {
            let index = self.context.new_rvalue_from_long(self.u64_type, i64::try_from(idx).expect("i64::try_from"));
            let element = self.context.new_array_access(None, aggregate_value, index);
            element.get_address(None)
        }
        else if value_type.is_vector().is_some() {
            panic!();
        }
        else if let Some(struct_type) = value_type.is_struct() {
            aggregate_value.access_field(None, struct_type.get_field(idx as i32)).to_rvalue()
        }
        else {
            panic!("Unexpected type {:?}", value_type);
        }
        /*assert_eq!(idx as c_uint as u64, idx);
        unsafe { llvm::LLVMBuildExtractValue(self.llbuilder, agg_val, idx as c_uint, UNNAMED) }*/
    }

    fn insert_value(&mut self, aggregate_value: RValue<'gcc>, value: RValue<'gcc>, idx: u64) -> RValue<'gcc> {
        // FIXME: it would be better if the API only called this on struct, not on arrays.
        assert_eq!(idx as usize as u64, idx);
        let value_type = aggregate_value.get_type();

        let lvalue =
            if value_type.is_array() {
                let index = self.context.new_rvalue_from_long(self.u64_type, i64::try_from(idx).expect("i64::try_from"));
                self.context.new_array_access(None, aggregate_value, index)
            }
            else if value_type.is_vector().is_some() {
                panic!();
            }
            else if let Some(struct_type) = value_type.is_struct() {
                aggregate_value.access_field(None, struct_type.get_field(idx as i32))
            }
            else {
                panic!("Unexpected type {:?}", value_type);
            };
        self.llbb().add_assignment(None, lvalue, value);

        aggregate_value
    }

    fn landing_pad(&mut self, ty: Type<'gcc>, pers_fn: RValue<'gcc>, num_clauses: usize) -> RValue<'gcc> {
        unimplemented!();
        /*unsafe {
            llvm::LLVMBuildLandingPad(self.llbuilder, ty, pers_fn, num_clauses as c_uint, UNNAMED)
        }*/
    }

    fn set_cleanup(&mut self, landing_pad: RValue<'gcc>) {
        unimplemented!();
        /*unsafe {
            llvm::LLVMSetCleanup(landing_pad, llvm::True);
        }*/
    }

    fn resume(&mut self, exn: RValue<'gcc>) -> RValue<'gcc> {
        unimplemented!();
        //unsafe { llvm::LLVMBuildResume(self.llbuilder, exn) }
    }

    fn cleanup_pad(&mut self, parent: Option<RValue<'gcc>>, args: &[RValue<'gcc>]) -> Funclet {
        unimplemented!();
        /*let name = const_cstr!("cleanuppad");
        let ret = unsafe {
            llvm::LLVMRustBuildCleanupPad(
                self.llbuilder,
                parent,
                args.len() as c_uint,
                args.as_ptr(),
                name.as_ptr(),
            )
        };
        Funclet::new(ret.expect("LLVM does not have support for cleanuppad"))*/
    }

    fn cleanup_ret(&mut self, funclet: &Funclet, unwind: Option<Block<'gcc>>) -> RValue<'gcc> {
        unimplemented!();
        /*let ret =
            unsafe { llvm::LLVMRustBuildCleanupRet(self.llbuilder, funclet.cleanuppad(), unwind) };
        ret.expect("LLVM does not have support for cleanupret")*/
    }

    fn catch_pad(&mut self, parent: RValue<'gcc>, args: &[RValue<'gcc>]) -> Funclet {
        unimplemented!();
        /*let name = const_cstr!("catchpad");
        let ret = unsafe {
            llvm::LLVMRustBuildCatchPad(
                self.llbuilder,
                parent,
                args.len() as c_uint,
                args.as_ptr(),
                name.as_ptr(),
            )
        };
        Funclet::new(ret.expect("LLVM does not have support for catchpad"))*/
    }

    fn catch_switch(&mut self, parent: Option<RValue<'gcc>>, unwind: Option<Block<'gcc>>, num_handlers: usize) -> RValue<'gcc> {
        unimplemented!();
        /*let name = const_cstr!("catchswitch");
        let ret = unsafe {
            llvm::LLVMRustBuildCatchSwitch(
                self.llbuilder,
                parent,
                unwind,
                num_handlers as c_uint,
                name.as_ptr(),
            )
        };
        ret.expect("LLVM does not have support for catchswitch")*/
    }

    fn add_handler(&mut self, catch_switch: RValue<'gcc>, handler: Block<'gcc>) {
        unimplemented!();
        /*unsafe {
            llvm::LLVMRustAddHandler(catch_switch, handler);
        }*/
    }

    fn set_personality_fn(&mut self, personality: RValue<'gcc>) {
        unimplemented!();
        /*unsafe {
            llvm::LLVMSetPersonalityFn(self.llfn(), personality);
        }*/
    }

    // Atomic Operations
    fn atomic_cmpxchg(&mut self, dst: RValue<'gcc>, cmp: RValue<'gcc>, src: RValue<'gcc>, order: AtomicOrdering, failure_order: AtomicOrdering, weak: bool) -> RValue<'gcc> {
        // TODO
        self.llbb().add_assignment(None, dst.dereference(None), src);
        /*let compare_exchange = self.context.get_builtin_function("__atomic_compare_exchange_n"); // TODO: replace n with number?
        let order = self.context.new_rvalue_from_int(self.i32_type, order.to_gcc());
        let failure_order = self.context.new_rvalue_from_int(self.i32_type, failure_order.to_gcc());
        let weak = self.context.new_rvalue_from_int(self.bool_type, weak as i32);
        self.context.new_call(None, compare_exchange, &[dst, cmp, src, weak, order, failure_order]);*/
        // FIXME: return the result of the call instead of a dummy value. Bug in libgccjit makes
        // __atomic_compare_exchange_n return void instead of bool.
        self.context.new_rvalue_from_int(self.bool_type, 1)
    }

    fn atomic_rmw(&mut self, op: AtomicRmwBinOp, dst: RValue<'gcc>, src: RValue<'gcc>, order: AtomicOrdering) -> RValue<'gcc> {
        let name =
            match op {
                AtomicRmwBinOp::AtomicXchg => "__atomic_exchange_n",
                AtomicRmwBinOp::AtomicAdd => {
                    // TODO: implement using atomics.
                    let temp = dst.dereference(None).to_rvalue();
                    println!("{:?}", dst);
                    self.llbb().add_assignment_op(None, dst.dereference(None), BinaryOp::Plus, src);
                    return temp;
                    //"__atomic_fetch_add" // TODO: try __atomic_fetch_add_N?
                },
                AtomicRmwBinOp::AtomicSub => "__atomic_fetch_sub",
                AtomicRmwBinOp::AtomicAnd => "__atomic_fetch_and",
                AtomicRmwBinOp::AtomicNand => "__atomic_fetch_nand",
                AtomicRmwBinOp::AtomicOr => "__atomic_fetch_or",
                AtomicRmwBinOp::AtomicXor => "__atomic_fetch_xor",
                AtomicRmwBinOp::AtomicMax => unimplemented!(),
                AtomicRmwBinOp::AtomicMin => unimplemented!(),
                AtomicRmwBinOp::AtomicUMax => unimplemented!(),
                AtomicRmwBinOp::AtomicUMin => unimplemented!(),
            };


        let atomic_function = self.context.get_builtin_function(name);
        let order = self.context.new_rvalue_from_int(self.i32_type, order.to_gcc());
        let res = self.context.new_call(None, atomic_function, &[dst, src, order]);
        // FIXME: return the return value of the call (res) instead of dst when I can cast the result to
        // the right type.
        dst.dereference(None).to_rvalue()
        //self.context.new_cast(None, res, src.get_type())
    }

    fn atomic_fence(&mut self, order: AtomicOrdering, scope: SynchronizationScope) {
        let name =
            match scope {
                SynchronizationScope::Other => "__atomic_thread_fence", // FIXME: not sure about this one.
                SynchronizationScope::SingleThread => "__atomic_signal_fence",
                SynchronizationScope::CrossThread => "__atomic_thread_fence",
            };
        let thread_fence = self.context.get_builtin_function(name);
        let order = self.context.new_rvalue_from_int(self.i32_type, order.to_gcc());
        self.llbb().add_eval(None, self.context.new_call(None, thread_fence, &[order]));
    }

    fn set_invariant_load(&mut self, load: RValue<'gcc>) {
        // NOTE: Hack to consider vtable function pointer as non-global-variable function pointer.
        self.normal_function_addresses.borrow_mut().insert(load);
        // TODO
        /*unsafe {
            llvm::LLVMSetMetadata(
                load,
                llvm::MD_invariant_load as c_uint,
                llvm::LLVMMDNodeInContext(self.cx.llcx, ptr::null(), 0),
            );
        }*/
    }

    fn lifetime_start(&mut self, ptr: RValue<'gcc>, size: Size) {
        // TODO
        //self.call_lifetime_intrinsic("llvm.lifetime.start.p0i8", ptr, size);
    }

    fn lifetime_end(&mut self, ptr: RValue<'gcc>, size: Size) {
        // TODO
        //self.call_lifetime_intrinsic("llvm.lifetime.end.p0i8", ptr, size);
    }

    fn call(&mut self, func: RValue<'gcc>, args: &[RValue<'gcc>], funclet: Option<&Funclet>) -> RValue<'gcc> {
        // FIXME: remove when having a proper API.
        let gcc_func = unsafe { std::mem::transmute(func) };
        if self.functions.borrow().values().find(|value| **value == gcc_func).is_some() {
            self.function_call(func, args, funclet)
        }
        else {
            self.function_ptr_call(func, args, funclet)
        }
    }

    fn zext(&mut self, value: RValue<'gcc>, dest_typ: Type<'gcc>) -> RValue<'gcc> {
        // FIXME: this does not zero-extend.
        if value.get_type().is_bool() && dest_typ.is_i8(&self.cx) {
            // FIXME: hack because base::from_immediate converts i1 to i8.
            // Fix the code in codegen_ssa::base::from_immediate.
            return value;
        }
        //println!("zext: {:?} -> {:?}", value, dest_typ);
        self.context.new_cast(None, value, dest_typ)
    }

    fn cx(&self) -> &CodegenCx<'gcc, 'tcx> {
        self.cx
    }

    unsafe fn delete_basic_block(&mut self, bb: Block<'gcc>) {
        unimplemented!();
        //llvm::LLVMDeleteBasicBlock(bb);
    }

    fn do_not_inline(&mut self, llret: RValue<'gcc>) {
        unimplemented!();
        //llvm::Attribute::NoInline.apply_callsite(llvm::AttributePlace::Function, llret);
    }
}

impl<'a, 'gcc, 'tcx> StaticBuilderMethods for Builder<'a, 'gcc, 'tcx> {
    fn get_static(&mut self, def_id: DefId) -> RValue<'gcc> {
        unimplemented!();
        // Forward to the `get_static` method of `CodegenCx`
        //self.cx().get_static(def_id)
    }
}

impl<'tcx> HasParamEnv<'tcx> for Builder<'_, '_, 'tcx> {
    fn param_env(&self) -> ParamEnv<'tcx> {
        self.cx.param_env()
    }
}

impl<'tcx> HasTargetSpec for Builder<'_, '_, 'tcx> {
    fn target_spec(&self) -> &Target {
        &self.cx.target_spec()
    }
}

trait ToGccComp {
    fn to_gcc_comparison(&self) -> ComparisonOp;
}

impl ToGccComp for IntPredicate {
    fn to_gcc_comparison(&self) -> ComparisonOp {
        match *self {
            IntPredicate::IntEQ => ComparisonOp::Equals,
            IntPredicate::IntNE => ComparisonOp::NotEquals,
            IntPredicate::IntUGT => ComparisonOp::GreaterThan,
            IntPredicate::IntUGE => ComparisonOp::GreaterThanEquals,
            IntPredicate::IntULT => ComparisonOp::LessThan,
            IntPredicate::IntULE => ComparisonOp::LessThanEquals,
            IntPredicate::IntSGT => ComparisonOp::GreaterThan,
            IntPredicate::IntSGE => ComparisonOp::GreaterThanEquals,
            IntPredicate::IntSLT => ComparisonOp::LessThan,
            IntPredicate::IntSLE => ComparisonOp::LessThanEquals,
        }
    }
}

impl ToGccComp for RealPredicate {
    fn to_gcc_comparison(&self) -> ComparisonOp {
        // TODO: check that ordered vs non-ordered is respected.
        match *self {
            RealPredicate::RealPredicateFalse => unreachable!(),
            RealPredicate::RealOEQ => ComparisonOp::Equals,
            RealPredicate::RealOGT => ComparisonOp::GreaterThan,
            RealPredicate::RealOGE => ComparisonOp::GreaterThanEquals,
            RealPredicate::RealOLT => ComparisonOp::LessThan,
            RealPredicate::RealOLE => ComparisonOp::LessThanEquals,
            RealPredicate::RealONE => ComparisonOp::NotEquals,
            RealPredicate::RealORD => unreachable!(),
            RealPredicate::RealUNO => unreachable!(),
            RealPredicate::RealUEQ => ComparisonOp::Equals,
            RealPredicate::RealUGT => ComparisonOp::GreaterThan,
            RealPredicate::RealUGE => ComparisonOp::GreaterThan,
            RealPredicate::RealULT => ComparisonOp::LessThan,
            RealPredicate::RealULE => ComparisonOp::LessThan,
            RealPredicate::RealUNE => ComparisonOp::NotEquals,
            RealPredicate::RealPredicateTrue => unreachable!(),
        }
    }
}

#[repr(C)]
enum MemOrdering {
    __ATOMIC_RELAXED,
    __ATOMIC_CONSUME,
    __ATOMIC_ACQUIRE,
    __ATOMIC_RELEASE,
    __ATOMIC_ACQ_REL,
    __ATOMIC_SEQ_CST,
}

trait ToGccOrdering {
    fn to_gcc(self) -> i32;
}

impl ToGccOrdering for AtomicOrdering {
    fn to_gcc(self) -> i32 {
        use MemOrdering::*;

        let ordering =
            match self {
                AtomicOrdering::NotAtomic => __ATOMIC_RELAXED, // TODO: check if that's the same.
                AtomicOrdering::Unordered => __ATOMIC_RELAXED,
                AtomicOrdering::Monotonic => __ATOMIC_RELAXED, // TODO: check if that's the same.
                AtomicOrdering::Acquire => __ATOMIC_ACQUIRE,
                AtomicOrdering::Release => __ATOMIC_RELEASE,
                AtomicOrdering::AcquireRelease => __ATOMIC_ACQ_REL,
                AtomicOrdering::SequentiallyConsistent => __ATOMIC_SEQ_CST,
            };
        ordering as i32
    }
}
