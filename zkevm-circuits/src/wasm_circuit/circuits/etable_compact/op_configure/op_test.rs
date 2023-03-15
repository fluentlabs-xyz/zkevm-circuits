use super::*;
use crate::wasm_circuit::{
    circuits::{
        mtable_compact::encode::MemoryTableLookupEncode,
        utils::{bn_to_field, Context},
    },
};
use crate::constant;
use halo2_proofs::{
    plonk::{Error, Expression, VirtualCells},
};
use eth_types::Field;
use crate::wasm_circuit::specs::{
    etable::EventTableEntry,
    itable::{OpcodeClass, OPCODE_ARG0_SHIFT, OPCODE_CLASS_SHIFT},
};
use crate::wasm_circuit::specs::{itable::TestOp, mtable::VarType};
use crate::wasm_circuit::specs::{itable::OPCODE_ARG1_SHIFT, step::StepInfo};

pub struct TestConfig {
    is_i64: BitCell,
    value_inv: UnlimitedCell,

    value: U64Cell,
    res: BitCell,

    lookup_stack_read: MTableLookupCell,
    lookup_stack_write: MTableLookupCell,
}

pub struct TestConfigBuilder {}

impl<F: Field> EventTableOpcodeConfigBuilder<F> for TestConfigBuilder {
    fn configure(
        common: &mut EventTableCellAllocator<F>,
        constraint_builder: &mut ConstraintBuilder<F>,
    ) -> Box<dyn EventTableOpcodeConfig<F>> {
        let value = common.alloc_u64();
        let res = common.alloc_bit_value();

        let is_i64 = common.alloc_bit_value();
        let value_inv = common.alloc_unlimited_value();

        let lookup_stack_read = common.alloc_mtable_lookup();
        let lookup_stack_write = common.alloc_mtable_lookup();

        constraint_builder.push(
            "test op value_is_zero",
            Box::new(move |meta| {
                vec![
                    res.expr(meta) * value.expr(meta),
                    value.expr(meta) * value_inv.expr(meta) - constant_from!(1) + res.expr(meta),
                ]
            }),
        );

        Box::new(TestConfig {
            is_i64,
            value_inv,
            value,
            res,
            lookup_stack_read,
            lookup_stack_write,
        })
    }
}

impl<F: Field> EventTableOpcodeConfig<F> for TestConfig {
    fn opcode(&self, meta: &mut VirtualCells<'_, F>) -> Expression<F> {
        let vtype = self.is_i64.expr(meta) * constant_from!(VarType::I64 as u64)
            + (constant_from!(1) - self.is_i64.expr(meta)) * constant_from!(VarType::I32 as u64);

        constant!(bn_to_field(
            &(BigUint::from(OpcodeClass::Test as u64) << OPCODE_CLASS_SHIFT)
        )) + constant_from!(TestOp::Eqz as u16)
            * constant!(bn_to_field(&(BigUint::from(1u64) << OPCODE_ARG0_SHIFT)))
            + vtype * constant!(bn_to_field(&(BigUint::from(1u64) << OPCODE_ARG1_SHIFT)))
    }

    fn assign(
        &self,
        ctx: &mut Context<'_, F>,
        step_info: &StepStatus,
        entry: &EventTableEntry,
    ) -> Result<(), Error> {
        match entry.step_info {
            StepInfo::Test {
                vtype,
                value,
                result,
            } => {
                self.is_i64.assign(ctx, vtype == VarType::I64)?;

                self.value.assign(ctx, value)?;
                self.value_inv
                    .assign(ctx, F::from(value).invert().unwrap_or(F::zero()))?;
                self.res.assign(ctx, result == 1)?;

                self.lookup_stack_read.assign(
                    ctx,
                    &MemoryTableLookupEncode::encode_stack_read(
                        BigUint::from(step_info.current.eid),
                        BigUint::from(1 as u64),
                        BigUint::from(step_info.current.sp + 1),
                        BigUint::from(vtype as u16),
                        BigUint::from(value),
                    ),
                )?;

                self.lookup_stack_write.assign(
                    ctx,
                    &MemoryTableLookupEncode::encode_stack_write(
                        BigUint::from(step_info.current.eid),
                        BigUint::from(2 as u64),
                        BigUint::from(step_info.current.sp + 1),
                        BigUint::from(VarType::I32 as u16),
                        BigUint::from(result as u32 as u64),
                    ),
                )?;
            }
            _ => unreachable!(),
        }

        Ok(())
    }

    fn mops(&self, _meta: &mut VirtualCells<'_, F>) -> Option<Expression<F>> {
        Some(constant_from!(2))
    }

    fn mtable_lookup(
        &self,
        meta: &mut VirtualCells<'_, F>,
        item: MLookupItem,
        common_config: &EventTableCommonConfig<F>,
    ) -> Option<Expression<F>> {
        match item {
            MLookupItem::First => Some(MemoryTableLookupEncode::encode_stack_read(
                common_config.eid(meta),
                constant_from!(1),
                common_config.sp(meta) + constant_from!(1),
                (constant_from!(1) - self.is_i64.expr(meta)) * constant_from!(VarType::I32)
                    + self.is_i64.expr(meta) * constant_from!(VarType::I64),
                self.value.expr(meta),
            )),

            MLookupItem::Second => Some(MemoryTableLookupEncode::encode_stack_write(
                common_config.eid(meta),
                constant_from!(2),
                common_config.sp(meta) + constant_from!(1),
                constant_from!(VarType::I32),
                self.res.expr(meta),
            )),

            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::wasm_circuit::test::test_circuit_noexternal;

    #[test]
    fn test_eqz() {
        let textual_repr = r#"
                (module
                    (func (export "test")
                      (i32.const 0)
                      (i32.eqz)
                      (drop)

                      (i32.const 1)
                      (i32.eqz)
                      (drop)

                      (i64.const 0)
                      (i64.eqz)
                      (drop)

                      (i64.const 1)
                      (i64.eqz)
                      (drop)
                    )
                   )
                "#;

        test_circuit_noexternal(textual_repr).unwrap()
    }
}