use super::*;
use crate::wasm_circuit::{circuits::utils::{bn_to_field, Context}, specs};
use halo2_proofs::{
    arithmetic::FieldExt,
    plonk::{Error, Expression, VirtualCells},
};
use eth_types::Field;
use crate::wasm_circuit::specs::{etable::EventTableEntry, itable::OPCODE_CLASS_SHIFT};
use crate::constant;

pub struct DropConfig {}

pub struct DropConfigBuilder {}

impl<F: Field> EventTableOpcodeConfigBuilder<F> for DropConfigBuilder {
    fn configure(
        _common: &mut EventTableCellAllocator<F>,
        _constraint_builder: &mut ConstraintBuilder<F>,
    ) -> Box<dyn EventTableOpcodeConfig<F>> {
        Box::new(DropConfig {})
    }
}

impl<F: Field> EventTableOpcodeConfig<F> for DropConfig {
    fn opcode(&self, _meta: &mut VirtualCells<'_, F>) -> Expression<F> {
        constant!(bn_to_field(
            &(BigUint::from(OpcodeClass::Drop as u64) << OPCODE_CLASS_SHIFT)
        ))
    }

    fn assign(
        &self,
        _ctx: &mut Context<'_, F>,
        _step: &StepStatus,
        entry: &EventTableEntry,
    ) -> Result<(), Error> {
        match &entry.step_info {
            specs::step::StepInfo::Drop => Ok(()),
            _ => unreachable!(),
        }
    }

    fn sp_diff(&self, _meta: &mut VirtualCells<'_, F>) -> Option<Expression<F>> {
        Some(constant!(F::one()))
    }
}