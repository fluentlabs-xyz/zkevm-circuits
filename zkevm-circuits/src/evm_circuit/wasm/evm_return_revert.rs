use crate::{
    evm_circuit::{
        execution::ExecutionGadget,
        param::{N_BYTES_MEMORY_ADDRESS, STACK_CAPACITY},
        step::ExecutionState,
        util::{
            common_gadget::RestoreContextGadget,
            constraint_builder::{
                ConstrainBuilderCommon, ReversionInfo, StepStateTransition,
                Transition::{Delta, To},
            },
            math_gadget::{IsZeroGadget, MinMaxGadget},
            not, CachedRegion, Cell,
        },
        witness::{Block, Call, ExecStep, Transaction},
    },
    table::{AccountFieldTag, CallContextFieldTag},
    util::Expr,
};

use bus_mapping::{circuit_input_builder::CopyDataType, evm::OpcodeId};
use eth_types::{Field, ToScalar, U256};
use ethers_core::utils::keccak256;
use halo2_proofs::{circuit::Value, plonk::Error};
use crate::evm_circuit::util::constraint_builder::EVMConstraintBuilder;


#[derive(Clone, Debug)]
pub(crate) struct EvmReturnRevertGadget<F> {
    opcode: Cell<F>,

    length: Cell<F>,
    offset: Cell<F>,

    is_success: Cell<F>,
    restore_context: RestoreContextGadget<F>,

    copy_length: MinMaxGadget<F, N_BYTES_MEMORY_ADDRESS>,
    copy_rw_increase: Cell<F>,
    copy_rw_increase_is_zero: IsZeroGadget<F>,

    return_data_offset: Cell<F>,
    return_data_length: Cell<F>,

    code_hash: Cell<F>,

    caller_id: Cell<F>,
    address: Cell<F>,
    reversion_info: ReversionInfo<F>,
}

impl<F: Field> ExecutionGadget<F> for EvmReturnRevertGadget<F> {
    const NAME: &'static str = "RETURN_REVERT";

    const EXECUTION_STATE: ExecutionState = ExecutionState::RETURN_REVERT;

    fn configure(cb: &mut EVMConstraintBuilder<F>) -> Self {
        let opcode = cb.query_cell();
        // TODO need a fix
        // cb.opcode_lookup(opcode.expr(), 0.expr());

        let length = cb.query_cell();
        let offset = cb.query_cell();
        cb.stack_pop(length.expr());
        cb.stack_pop(offset.expr());

        let is_success = cb.call_context(None, CallContextFieldTag::IsSuccess);
        cb.require_boolean("is_success is boolean", is_success.expr());
        cb.require_equal(
            "if is_success, opcode is RETURN. if not, opcode is REVERT",
            opcode.expr(),
            is_success.expr() * OpcodeId::RETURN.expr()
                + not::expr(is_success.expr()) * OpcodeId::REVERT.expr(),
        );

        // There are 4 cases non-mutually exclusive, A to D, to handle, depending on if
        // the call is, or is not, a create, root, or successful. See the specs at
        // https://github.com/privacy-scaling-explorations/zkevm-specs/blob/master/specs/opcode/F3RETURN_FDREVERT.md
        // for more details.
        let is_create = cb.curr.state.is_create.expr();
        let is_root = cb.curr.state.is_root.expr();

        // These are globally defined because they are used across multiple cases.
        let copy_rw_increase = cb.query_cell();
        let copy_rw_increase_is_zero = IsZeroGadget::construct(cb, copy_rw_increase.expr());

        // Case A in the specs.
        cb.condition(is_create.clone() * is_success.expr(), |cb| {
            cb.require_equal(
                "increase rw counter once for each memory to bytecode byte copied",
                copy_rw_increase.expr(),
                length.expr(),
            );
        });

        let is_contract_deployment =
            is_create.clone() * is_success.expr() * not::expr(copy_rw_increase_is_zero.expr());
        let (caller_id, address, reversion_info, code_hash) =
            cb.condition(is_contract_deployment.clone(), |cb| {
                // We don't need to place any additional constraints on code_hash because the
                // copy circuit enforces that it is the hash of the bytes in the copy lookup.
                let code_hash = cb.query_cell_phase2();
                cb.copy_table_lookup(
                    cb.curr.state.call_id.expr(),
                    CopyDataType::Memory.expr(),
                    code_hash.expr(),
                    CopyDataType::Bytecode.expr(),
                    offset.expr(),
                    offset.expr() + length.expr(),
                    0.expr(),
                    length.expr(),
                    0.expr(),
                    copy_rw_increase.expr(),
                );

                let [caller_id, address] = [
                    CallContextFieldTag::CallerId,
                    CallContextFieldTag::CalleeAddress,
                ]
                    .map(|tag| cb.call_context(None, tag));
                let mut reversion_info = cb.reversion_info_read(None);

                cb.account_write(
                    address.expr(),
                    AccountFieldTag::CodeHash,
                    code_hash.expr(),
                    cb.empty_code_hash_rlc(),
                    Some(&mut reversion_info),
                );

                (caller_id, address, reversion_info, code_hash)
            });

        // Case B in the specs.
        cb.condition(is_root.expr(), |cb| {
            cb.require_next_state(ExecutionState::EndTx);
            cb.call_context_lookup(
                false.expr(),
                None,
                CallContextFieldTag::IsPersistent,
                is_success.expr(),
            );
            cb.require_step_state_transition(StepStateTransition {
                program_counter: To(0.expr()),
                stack_pointer: To(STACK_CAPACITY.expr()),
                rw_counter: Delta(
                    cb.rw_counter_offset()
                        + not::expr(is_success.expr())
                        * cb.curr.state.reversible_write_counter.expr(),
                ),
                // gas_left: Delta(-memory_expansion.gas_cost()),
                reversible_write_counter: To(0.expr()),
                memory_word_size: To(0.expr()),
                ..StepStateTransition::default()
            });
        });

        // Case C in the specs.
        let restore_context = cb.condition(not::expr(is_root.expr()), |cb| {
            RestoreContextGadget::construct(
                cb,
                is_success.expr(),
                not::expr(is_create.clone()) * (2.expr() + copy_rw_increase.expr()),
                offset.expr(),
                length.expr(),
                0.expr(),
                is_contract_deployment, // There is one reversible write in this case.
            )
        });

        // Case D in the specs.
        let (return_data_offset, return_data_length, copy_length) = cb.condition(
            not::expr(is_create.clone()) * not::expr(is_root.clone()),
            |cb| {
                let [return_data_offset, return_data_length] = [
                    CallContextFieldTag::ReturnDataOffset,
                    CallContextFieldTag::ReturnDataLength,
                ]
                    .map(|field_tag| cb.call_context(None, field_tag));
                let copy_length =
                    MinMaxGadget::construct(cb, return_data_length.expr(), length.expr());
                cb.require_equal(
                    "increase rw counter twice for each memory to memory byte copied",
                    copy_length.min() + copy_length.min(),
                    copy_rw_increase.expr(),
                );
                (return_data_offset, return_data_length, copy_length)
            },
        );
        cb.condition(
            not::expr(is_create.clone())
                * not::expr(is_root.clone())
                * not::expr(copy_rw_increase_is_zero.expr()),
            |cb| {
                cb.copy_table_lookup(
                    cb.curr.state.call_id.expr(),
                    CopyDataType::Memory.expr(),
                    cb.next.state.call_id.expr(),
                    CopyDataType::Memory.expr(),
                    offset.expr(),
                    offset.expr() + length.expr(),
                    return_data_offset.expr(),
                    copy_length.min(),
                    0.expr(),
                    copy_rw_increase.expr(),
                );
            },
        );

        // Without this, copy_rw_increase would be unconstrained for non-create root
        // calls.
        cb.condition(not::expr(is_create) * is_root, |cb| {
            cb.require_zero(
                "rw counter is 0 if there is no copy event",
                copy_rw_increase.expr(),
            );
        });

        Self {
            opcode,
            length,
            offset,
            is_success,
            copy_length,
            copy_rw_increase,
            copy_rw_increase_is_zero,
            return_data_offset,
            return_data_length,
            restore_context,
            code_hash,
            address,
            caller_id,
            reversion_info,
        }
    }

    fn assign_exec_step(
        &self,
        region: &mut CachedRegion<'_, '_, F>,
        offset: usize,
        block: &Block<F>,
        _: &Transaction,
        call: &Call,
        step: &ExecStep,
    ) -> Result<(), Error> {
        self.opcode.assign(
            region,
            offset,
            Value::known(F::from(step.opcode.unwrap().as_u64())),
        )?;

        let [length, memory_offset] = [0, 1].map(|i| block.rws[step.rw_indices[i]].stack_value());
        self.length.assign(region, offset, Value::<F>::known(length.to_scalar().unwrap()))?;
        self.offset.assign(region, offset, Value::<F>::known(memory_offset.to_scalar().unwrap()))?;

        self.is_success
            .assign(region, offset, Value::known(call.is_success.into()))?;

        if !call.is_root && !call.is_create {
            for (cell, value) in [
                (&self.return_data_length, call.return_data_length.into()),
                (&self.return_data_offset, call.return_data_offset.into()),
            ] {
                cell.assign(region, offset, Value::known(value))?;
            }

            self.copy_length.assign(
                region,
                offset,
                F::from(call.return_data_length),
                F::from(length.as_u64()),
            )?;
        }

        if call.is_create && call.is_success {
            let values: Vec<_> = (3..3 + length.as_usize())
                .map(|i| block.rws[step.rw_indices[i]].memory_value())
                .collect();
            let mut code_hash = keccak256(&values);
            code_hash.reverse();
            self.code_hash.assign(
                region,
                offset,
                region.word_rlc(U256::from_little_endian(&code_hash)),
            )?;
        }

        let copy_rw_increase = if call.is_create && call.is_success {
            length.as_u64()
        } else if !call.is_root {
            2 * std::cmp::min(call.return_data_length, length.as_u64())
        } else {
            0
        };
        self.copy_rw_increase
            .assign(region, offset, Value::known(F::from(copy_rw_increase)))?;
        self.copy_rw_increase_is_zero
            .assign(region, offset, F::from(copy_rw_increase))?;

        let is_contract_deployment = call.is_create && call.is_success && !length.is_zero();
        if !call.is_root {
            let rw_counter_offset = 3 + if is_contract_deployment {
                5 + length.as_u64()
            } else {
                0
            };
            self.restore_context.assign(
                region,
                offset,
                block,
                call,
                step,
                rw_counter_offset.try_into().unwrap(),
            )?;
        }

        self.caller_id.assign(
            region,
            offset,
            Value::known(call.caller_id.to_scalar().unwrap()),
        )?;

        self.address.assign(
            region,
            offset,
            Value::known(call.callee_address.to_scalar().unwrap()),
        )?;

        self.reversion_info.assign(
            region,
            offset,
            call.rw_counter_end_of_reversion,
            call.is_persistent,
        )?;

        Ok(())
    }
}

#[cfg(test)]
mod test {
    use std::fs;
    use ethers_core::k256::pkcs8::der::Encode;
    use itertools::Itertools;

    use eth_types::{address, bytecode, evm_types::OpcodeId, geth_types::{Account, GethData}, Address, Bytecode, Word, bytecode_internal, StackWord};
    use eth_types::bytecode::{UncheckedWasmBinary, WasmBinaryBytecode};
    use mock::{eth, MOCK_ACCOUNTS, TestContext};

    use crate::test_util::CircuitTestBuilder;

    const CALLEE_ADDRESS: Address = Address::repeat_byte(0xff);
    const CALLER_ADDRESS: Address = Address::repeat_byte(0x34);

    fn callee_bytecode(is_return: bool, offset: u32, length: u32) -> Bytecode {
        let mut code = bytecode! {
            I32Const[length]
            I32Const[offset]
        };
        code.write_op(if is_return {
            OpcodeId::RETURN
        } else {
            OpcodeId::REVERT
        });
        code
    }

    fn caller_bytecode(_return_data_offset: u64, return_data_length: u64) -> Bytecode {
        let mut code = Bytecode::default();
        code.with_global_data(0, 0x7f, CALLER_ADDRESS.to_fixed_bytes().to_vec().unwrap());
        bytecode_internal!(code,
            I32Const[return_data_length]
            I32Const[0] // call data length
            I32Const[0] // call data offset
            I32Const[0] // value offset
            I32Const[0x7f] // offset of CALLER_ADDRESS
            I32Const[4000] // gas
            CALL
            STOP
        );
        code
    }

    #[test]
    fn test_return_hello_world() {
        let mut code = Bytecode::default();
        code.with_global_data(0, 0, "Hello, World".as_bytes().to_vec());
        bytecode_internal!(code,
            I32Const[0]
            I32Const[12]
            RETURN
        );
        CircuitTestBuilder::new_from_test_ctx(
            TestContext::<2, 1>::simple_ctx_with_bytecode(code).unwrap(),
        ).run();
    }

    #[test]
    fn test_deploy_hello_world() {
        let wasm_bin = fs::read("./deploy.wasm").unwrap();
        let code = UncheckedWasmBinary::from(wasm_bin);
        CircuitTestBuilder::new_from_test_ctx(
            TestContext::<2, 1>::simple_ctx_with_bytecode(code).unwrap(),
        ).run();
    }

    #[test]
    fn test_revert_hello_world() {
        let mut code = Bytecode::default();
        code.with_global_data(0, 0, "Hello, World".as_bytes().to_vec());
        bytecode_internal!(code,
            I32Const[0]
            I32Const[12]
            REVERT
        );
        CircuitTestBuilder::new_from_test_ctx(
            TestContext::<2, 1>::simple_ctx_with_bytecode(code).unwrap(),
        ).run();
    }

    #[test]
    fn test_return_root_noncreate() {
        let test_parameters = [(0, 0), (0, 10)]; //TODO, (300, 20), (1000, 0)];
        for ((offset, length), is_return) in test_parameters.iter().cartesian_product(&[true, false])
        {
            let code = callee_bytecode(*is_return, *offset, *length);
            if *is_return {
                CircuitTestBuilder::new_from_test_ctx(
                    TestContext::<2, 1>::simple_ctx_with_bytecode(code).unwrap(),
                ).run();
            } else {
                CircuitTestBuilder::new_from_test_ctx(
                    TestContext::<2, 1>::simple_ctx_with_bytecode(code).unwrap(),
                ).run();
            }
        }
    }

    #[test]
    #[ignore]
    fn test_return_nonroot_noncreate() {
        let test_parameters = [
            ((0, 0), (0, 0)),
            ((0, 10), (0, 10)),
            ((0, 10), (0, 20)),
            ((0, 20), (0, 10)),
            ((64, 1), (0, 10)), // Expands memory in RETURN/REVERT opcode
            ((0, 10), (1000, 0)),
            ((1000, 0), (0, 10)),
            ((1000, 0), (1000, 0)),
        ];
        for (((callee_offset, callee_length), (caller_offset, caller_length)), is_return) in
        test_parameters.iter().cartesian_product(&[true, false])
        {
            let callee = Account {
                address: CALLEE_ADDRESS,
                code: callee_bytecode(*is_return, *callee_offset, *callee_length).into(),
                nonce: Word::one(),
                ..Default::default()
            };
            let caller = Account {
                address: CALLER_ADDRESS,
                code: caller_bytecode(*caller_offset, *caller_length).into(),
                nonce: Word::one(),
                ..Default::default()
            };

            let ctx = TestContext::<3, 1>::new(
                None,
                |accs| {
                    accs[0]
                        .address(address!("0x000000000000000000000000000000000000cafe"))
                        .balance(Word::from(10u64.pow(19)));
                    accs[1].account(&caller);
                    accs[2].account(&callee);
                },
                |mut txs, accs| {
                    txs[0]
                        .from(accs[0].address)
                        .to(accs[1].address)
                        .gas(100000u64.into());
                },
                |block, _tx| block.number(0xcafeu64),
            )
                .unwrap();

            CircuitTestBuilder::new_from_test_ctx(ctx).run();
        }
    }

    #[test]
    fn test_return_root_create() {
        let test_parameters = [(0, 0), (0, 10)]; // TODO, (300, 20), (1000, 0)];
        for ((offset, length), is_return) in test_parameters.iter().cartesian_product(&[true, false])
        {
            let tx_input = callee_bytecode(*is_return, *offset, *length).wasm_binary();
            let ctx = if *is_return {
                TestContext::<1, 1>::new(
                    None,
                    |accs| {
                        accs[0].address(MOCK_ACCOUNTS[0]).balance(eth(10));
                    },
                    |mut txs, accs| {
                        txs[0].from(accs[0].address).input(tx_input.into());
                    },
                    |block, _| block,
                ).unwrap()
            } else {
                TestContext::<1, 1>::new(
                    None,
                    |accs| {
                        accs[0].address(MOCK_ACCOUNTS[0]).balance(eth(10));
                    },
                    |mut txs, accs| {
                        txs[0].from(accs[0].address).input(tx_input.into());
                    },
                    |block, _| block,
                ).unwrap()
            };

            CircuitTestBuilder::new_from_test_ctx(ctx).run();
        }
    }

    #[test]
    #[ignore]
    fn test_return_nonroot_create() {
        let test_parameters = [(0, 0), (0, 10), (300, 20), (1000, 0)];
        for ((offset, length), is_return) in
        test_parameters.iter().cartesian_product(&[true, false])
        {
            let initializer = callee_bytecode(*is_return, *offset, *length).wasm_binary();

            let root_code = bytecode! {
                PUSH32(Word::from_big_endian(&initializer))
                PUSH1(0)
                MSTORE

                PUSH1(initializer.len())        // size
                PUSH1(32 - initializer.len())   // offset
                PUSH1(0)                        // value

                CREATE
            };

            let caller = Account {
                address: CALLER_ADDRESS,
                code: root_code.into(),
                nonce: Word::one(),
                balance: eth(10),
                ..Default::default()
            };

            let ctx = TestContext::<2, 1>::new(
                None,
                |accs| {
                    accs[0]
                        .address(address!("0x000000000000000000000000000000000000cafe"))
                        .balance(eth(10));
                    accs[1].account(&caller);
                },
                |mut txs, accs| {
                    txs[0]
                        .from(accs[0].address)
                        .to(accs[1].address)
                        .gas(100000u64.into());
                },
                |block, _| block,
            )
                .unwrap();

            CircuitTestBuilder::new_from_test_ctx(ctx).run();
        }
    }

    #[test]
    #[ignore]
    fn test_nonpersistent_nonroot_create() {
        // Test the case where the initialization call is successful, but the CREATE
        // call is reverted.
        let initializer = callee_bytecode(true, 0, 10).wasm_binary();

        let root_code = bytecode! {
            PUSH32(Word::from_big_endian(&initializer))
            PUSH1(0)
            MSTORE

            PUSH1(initializer.len())        // size
            PUSH1(32 - initializer.len())   // offset
            PUSH1(0)                        // value

            CREATE
            PUSH1(0)
            PUSH1(0)
            REVERT
        };

        let caller = Account {
            address: CALLER_ADDRESS,
            code: root_code.into(),
            nonce: Word::one(),
            balance: eth(10),
            ..Default::default()
        };

        let ctx = TestContext::<2, 1>::new(
            None,
            |accs| {
                accs[0]
                    .address(address!("0x000000000000000000000000000000000000cafe"))
                    .balance(eth(10));
                accs[1].account(&caller);
            },
            |mut txs, accs| {
                txs[0]
                    .from(accs[0].address)
                    .to(accs[1].address)
                    .gas(100000u64.into());
            },
            |block, _| block,
        ).unwrap();

        CircuitTestBuilder::new_from_test_ctx(ctx).run();
    }

    #[test]
    // test CREATE/CREATE2 returndatasize both 0 for successful case
    fn test_return_nonroot_create_returndatasize() {
        let initializer = callee_bytecode(true, 0, 10).code();

        let mut bytecode = bytecode! {
             // CREATE + RETURNDATASIZE + RETURNDATACOPY logic
            PUSH32(Word::from_big_endian(&initializer))
            PUSH1(0)
            MSTORE

            PUSH1(initializer.len())        // size
            PUSH1(32 - initializer.len())   // offset
            PUSH1(0)                        // value
            CREATE
            RETURNDATASIZE
            PUSH1(0) // offset
            PUSH1(0) // dest offset
            RETURNDATACOPY // test return data copy
        };

        // CREATE2 logic
        // let code_creator: Vec<u8> = initializer
        //     .to_vec()
        //     .iter()
        //     .cloned()
        //     .chain(0u8..((32 - initializer.len() % 32) as u8))
        //     .collect();
        // for (index, word) in code_creator.chunks(32).enumerate() {
        //     bytecode.push(32, Word::from_big_endian(word));
        //     bytecode.push(32, Word::from(index * 32));
        //     bytecode.write_op(OpcodeId::MSTORE);
        // }
        bytecode.append(&bytecode! {
            PUSH3(0x123456) // salt
            PUSH1(initializer.len()) // length
            PUSH1(0) // offset
            PUSH1(0) // value
            CREATE2
            RETURNDATASIZE
            PUSH1(0) // offset
            PUSH1(0) // dest offset
            RETURNDATACOPY
        });

        let block: GethData = TestContext::<2, 1>::simple_ctx_with_bytecode(bytecode.clone())
            .unwrap()
            .into();

        // collect return opcode, retrieve next step, assure both contract create
        // successfully
        let created_contract_addr = block.geth_traces[0]
            .struct_logs
            .iter()
            .enumerate()
            .filter(|(_, s)| s.op == OpcodeId::RETURN)
            .flat_map(|(index, _)| block.geth_traces[0].struct_logs.get(index + 1))
            .flat_map(|s| s.stack.nth_last(0)) // contract addr on stack top
            .collect_vec();
        assert!(created_contract_addr.len() == 2); // both contract addr exist
        created_contract_addr
            .iter()
            .for_each(|addr| assert!(addr > &StackWord::zero()));

        // collect return opcode, retrieve next step, assure both returndata size is 0
        let return_data_size = block.geth_traces[0]
            .struct_logs
            .iter()
            .enumerate()
            .filter(|(_, s)| s.op == OpcodeId::RETURNDATASIZE)
            .flat_map(|(index, _)| block.geth_traces[0].struct_logs.get(index + 1))
            .flat_map(|s| s.stack.nth_last(0)) // returndata size on stack top
            .collect_vec();
        assert!(return_data_size.len() == 2);
        return_data_size
            .iter()
            .for_each(|size| assert_eq!(size, &StackWord::zero()));

        let text_ctx = TestContext::<2, 1>::simple_ctx_with_bytecode(bytecode).unwrap();
        CircuitTestBuilder::new_from_test_ctx(text_ctx).run();
    }
}