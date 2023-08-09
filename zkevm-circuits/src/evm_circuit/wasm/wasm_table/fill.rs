use halo2_proofs::circuit::Value;
use halo2_proofs::plonk::Error;

use bus_mapping::evm::OpcodeId;
use eth_types::{Field, ToScalar};

use crate::{
    evm_circuit::{
        execution::ExecutionGadget,
        step::ExecutionState,
        util::{
            CachedRegion,
            common_gadget::SameContextGadget,
            constraint_builder::{ConstrainBuilderCommon, StepStateTransition, Transition::Delta},
        },
        witness::{Block, Call, ExecStep, Transaction},
    },
    util::Expr,
};
use crate::evm_circuit::util::Cell;
use crate::evm_circuit::util::constraint_builder::EVMConstraintBuilder;

#[derive(Clone, Debug)]
pub(crate) struct WasmTableFillGadget<F> {
    same_context: SameContextGadget<F>,
    table_index: Cell<F>,
    start: Cell<F>,
    value: Cell<F>,
    range: Cell<F>,
}

impl<F: Field> ExecutionGadget<F> for WasmTableFillGadget<F> {
    const NAME: &'static str = "WASM_TABLE_FILL";

    const EXECUTION_STATE: ExecutionState = ExecutionState::WASM_TABLE_FILL;

    fn configure(cb: &mut EVMConstraintBuilder<F>) -> Self {
        let opcode = cb.query_cell();

        let table_index = cb.query_cell();
        let start = cb.query_cell();
        let value = cb.query_cell();
        let range = cb.query_cell();

        cb.stack_pop(start.expr());
        cb.stack_pop(value.expr());
        cb.stack_pop(range.expr());

/*
        cb.condition(is_fill_op.expr(), |cb| {
            cb.stack_pop(start.expr());
            cb.stack_pop(value.expr());
            cb.stack_pop(range.expr());
            cb.table_fill(table_index.expr(), start.expr(), value.expr(), range.expr());
        });
*/

        let step_state_transition = StepStateTransition {
            rw_counter: Delta(3.expr()),
            program_counter: Delta(1.expr()),
            stack_pointer: Delta(1.expr()),
            gas_left: Delta(-OpcodeId::TableGet.constant_gas_cost().expr()),
            ..Default::default()
        };

        let same_context = SameContextGadget::construct(cb, opcode, step_state_transition);

        Self {
            same_context,
            table_index,
            start,
            value,
            range,
        }
    }

    fn assign_exec_step(
        &self,
        region: &mut CachedRegion<'_, '_, F>,
        offset: usize,
        block: &Block<F>,
        _: &Transaction,
        _call: &Call,
        step: &ExecStep,
    ) -> Result<(), Error> {
        self.same_context.assign_exec_step(region, offset, step)?;

        let [start, value, range] =
            [step.rw_indices[0], step.rw_indices[1], step.rw_indices[2]]
            .map(|idx| block.rws[idx].stack_value());
        self.start.assign(region, offset, Value::<F>::known(start.to_scalar().unwrap()))?;
        self.value.assign(region, offset, Value::<F>::known(value.to_scalar().unwrap()))?;
        self.range.assign(region, offset, Value::<F>::known(range.to_scalar().unwrap()))?;

        match step.opcode.unwrap() {
            OpcodeId::TableFill => (),
            _ => unreachable!("not supported opcode: {:?}", step.opcode),
        };

        Ok(())
    }
}

#[cfg(test)]
mod test {
    use eth_types::{bytecode, Bytecode};
    use eth_types::bytecode::{TableDecl, WasmBinaryBytecode};
    use eth_types::evm_types::OpcodeId::I32Const;
    use mock::TestContext;

    use crate::test_util::CircuitTestBuilder;

    fn run_test(bytecode: Bytecode) {
        CircuitTestBuilder::new_from_test_ctx(
            TestContext::<2, 1>::simple_ctx_with_bytecode(bytecode).unwrap(),
        ).run()
    }

    #[test]
    fn test_table_set_get() {
        let mut code = bytecode! {
            I32Const[0]
            I64Const[0xff]
            I32Const[2]
            TableFill[0]
            Drop
        };
        code.with_table_decl(TableDecl::default_i32());
        run_test(code);
    }

}