use super::{Expr, ExprBuilder, LuaError, Name, SsaOp, SsaRef, ident_from_string_expr};

impl ExprBuilder<'_> {
    /// Build a call expression from already-versioned call parts.
    ///
    /// # Errors
    ///
    /// Returns any error [`Self::call_expr_with_arg_count`] returns.
    pub fn call_expr(&mut self, func: SsaRef, args: &[SsaRef], pc: i32) -> Result<Expr, LuaError> {
        let arg_count = i32::try_from(args.len() + 1).unwrap_or(i32::MAX);
        self.call_expr_with_arg_count(func, args, arg_count, pc)
    }

    /// Build a call expression, preserving open vs fixed argument semantics.
    ///
    /// # Errors
    ///
    /// Returns [`LuaError::Unsupported`] when the callee or an argument has no
    /// expression form at `pc` — an unresolved register, or a multi-value
    /// producer used where exactly one value is required.
    pub fn call_expr_with_arg_count(
        &mut self,
        func: SsaRef,
        args: &[SsaRef],
        arg_count: i32,
        pc: i32,
    ) -> Result<Expr, LuaError> {
        if let Some((receiver, method)) = self.method_receiver(func, pc)? {
            let call_args = args.iter().skip(1).copied().collect::<Vec<_>>();
            let args = self.call_args_exprs(&call_args, arg_count, pc)?;
            return Ok(Expr::Call {
                func: Box::new(receiver),
                args,
                method: Some(method),
            });
        }

        let func = self.expr_for_ref(func, pc)?;
        let args = self.call_args_exprs(args, arg_count, pc)?;
        Ok(Expr::Call {
            func: Box::new(func),
            args,
            method: None,
        })
    }

    /// Convert a value in a last-position fixed context to exactly one value.
    ///
    /// # Errors
    ///
    /// Returns any error [`Self::expr_for_ref`] returns for `reference`.
    pub fn expr_for_fixed_last_ref(
        &mut self,
        reference: SsaRef,
        use_pc: i32,
    ) -> Result<Expr, LuaError> {
        let expr = self.expr_for_ref(reference, use_pc)?;
        if self.last_position_needs_adjustment(reference) {
            Ok(Expr::Paren(Box::new(expr)))
        } else {
            Ok(expr)
        }
    }

    pub(super) fn call_args_exprs(
        &mut self,
        args: &[SsaRef],
        arg_count: i32,
        pc: i32,
    ) -> Result<Vec<Expr>, LuaError> {
        let fixed_count = arg_count != 0;
        let last_index = args.len().saturating_sub(1);
        args.iter()
            .copied()
            .enumerate()
            .map(|(index, arg)| {
                if fixed_count && index == last_index {
                    self.expr_for_fixed_last_ref(arg, pc)
                } else {
                    self.expr_for_ref(arg, pc)
                }
            })
            .collect()
    }

    pub(super) fn method_receiver(
        &mut self,
        func: SsaRef,
        pc: i32,
    ) -> Result<Option<(Expr, Name)>, LuaError> {
        let Some(node_id) = self.analysis.def_site(func) else {
            return Ok(None);
        };
        let Some(node) = self.analysis.node(self.function, node_id) else {
            return Ok(None);
        };
        let SsaOp::SelfOp { table, key, .. } = &node.op else {
            return Ok(None);
        };
        let key_expr = self.expr_for_ref(*key, pc)?;
        let Some(method) = ident_from_string_expr(&key_expr) else {
            return Ok(None);
        };
        let receiver = self.expr_for_ref(*table, pc)?;
        Ok(Some((receiver, method)))
    }
}
