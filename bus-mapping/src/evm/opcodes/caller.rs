use super::Opcode;
use crate::circuit_input_builder::{CircuitInputStateRef, ExecStep};
use crate::operation::CallContextField;
use crate::Error;
use eth_types::{GethExecStep, ToU256, U256};
use eth_types::evm_types::MemoryAddress;

/// Placeholder structure used to implement [`Opcode`] trait over it
/// corresponding to the [`OpcodeId::PC`](crate::evm::OpcodeId::PC) `OpcodeId`.
#[derive(Debug, Copy, Clone)]
pub(crate) struct Caller;

impl Opcode for Caller {
    fn gen_associated_ops(
        state: &mut CircuitInputStateRef,
        geth_steps: &[GethExecStep],
    ) -> Result<Vec<ExecStep>, Error> {
        let step = &geth_steps[0];
        let mut exec_step = state.new_step(step)?;
        let second_step = &geth_steps[1];
        let address = &second_step.memory.0;

        state.call_context_read(
            &mut exec_step,
            state.call()?.call_id,
            CallContextField::CallerAddress,
            U256::from_big_endian(address),
        );

        // Read dest offset as the last stack element
        let dest_offset = step.stack.nth_last(0)?;
        state.stack_read(&mut exec_step, step.stack.nth_last_filled(0), dest_offset)?;
        let offset_addr = MemoryAddress::try_from(dest_offset)?;

        // Copy result to memory
        for i in 0..20 {
            state.memory_write(&mut exec_step, offset_addr.map(|a| a + i), address[i])?;
        }
        let call_ctx = state.call_ctx_mut()?;
        call_ctx.memory = second_step.memory.clone();

        Ok(vec![exec_step])
    }
}

#[cfg(test)]
mod caller_tests {
    use std::fs;
    use super::*;
    use crate::{
        circuit_input_builder::ExecState, mock::BlockData, operation::CallContextOp,
        operation::StackOp, operation::RW,
    };
    use eth_types::{bytecode, evm_types::{OpcodeId, StackAddress}, geth_types::GethData, ToWord, Word};

    use mock::test_ctx::{helpers::*, TestContext};
    use pretty_assertions::assert_eq;
    use crate::operation::MemoryOp;

    #[test]
    fn caller_opcode_impl() {
        let code = bytecode! {
            I32Const[0x77]
            CALLER
        };
        // fs::write("/home/bfday/gitANKR/wasm0/zkwasm-circuits/tmp/w.wasm", code.wasm_binary());

        // Get the execution steps from the external tracer
        let block: GethData = TestContext::<2, 1>::new(
            None,
            account_0_code_account_1_no_code(code),
            tx_from_1_to_0,
            |block, _tx| block.number(0xcafeu64),
        )
        .unwrap()
        .into();
        let mut builder = BlockData::new_from_geth_data(block.clone()).new_circuit_input_builder();
        builder
            .handle_block(&block.eth_block, &block.geth_traces)
            .unwrap();
        let step = builder.block.txs()[0]
            .steps()
            .iter()
            .find(|step| step.exec_state == ExecState::Op(OpcodeId::CALLER))
            .unwrap();

        let call_id = builder.block.txs()[0].calls()[0].call_id;
        let caller_address = block.eth_block.transactions[0].from;
        let caller_address_bytes = caller_address.to_fixed_bytes();
        assert_eq!(step.bus_mapping_instance.len(), 22);
        assert_eq!(
            {
                let operation =
                    &builder.block.container.call_context[step.bus_mapping_instance[0].as_usize()];
                (operation.rw(), operation.op())
            },
            (
                RW::READ,
                &CallContextOp {
                    call_id,
                    field: CallContextField::CallerAddress,
                    value: caller_address.to_u256(),
                }
            )
        );
        assert_eq!(
            {
                let operation =
                    &builder.block.container.stack[step.bus_mapping_instance[1].as_usize()];
                (operation.rw(), operation.op())
            },
            (
                RW::WRITE,
                &StackOp::new(1, StackAddress::from(1023), Word::from(0x77))
            )
        );
        for idx in 0..20 {
            assert_eq!(
                {
                    let operation =
                        &builder.block.container.memory[step.bus_mapping_instance[2 + idx].as_usize()];
                    (operation.rw(), operation.op())
                },
                (
                    RW::WRITE,
                    &MemoryOp::new(1, MemoryAddress::from(0x77 + idx), caller_address_bytes[idx])
                )
            );
        }
    }
}
