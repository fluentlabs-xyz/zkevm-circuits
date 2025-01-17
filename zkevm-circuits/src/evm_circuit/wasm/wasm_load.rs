use halo2_proofs::circuit::Value;
use halo2_proofs::plonk::{Error, Expression};

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

#[derive(Clone, Debug)]
pub(crate) struct WasmLoadGadget<F> {
    same_context: SameContextGadget<F>,

    opcode_load_offset: Cell<F>,

    load_start_block_index: Cell<F>,
    load_start_block_inner_offset: Cell<F>,
    load_start_block_inner_offset_helper: Cell<F>,

    load_end_block_index: Cell<F>,
    load_end_block_inner_offset: Cell<F>,
    load_end_block_inner_offset_helper: Cell<F>,

    load_value1: Cell<F>,
    load_value2: Cell<F>,

    mask_bits: [Cell<F>; 16],
    offset_modulus: Cell<F>,
    res: Cell<F>,
    value_in_heap: Cell<F>,
    load_base: Cell<F>,

    vtype: Cell<F>,
    is_one_byte: Cell<F>,
    is_two_bytes: Cell<F>,
    is_four_bytes: Cell<F>,
    is_eight_bytes: Cell<F>,
    is_sign: Cell<F>,
    is_i64: Cell<F>,

    highest_u4: [Cell<F>; 4],

    //lookup_offset_len_bits: OffsetLenBitsTableLookupCell,
    //lookup_pow: PowTableLookupCell,

    address_within_allocated_pages_helper: Cell<F>,
}

impl<F: Field> ExecutionGadget<F> for WasmLoadGadget<F> {
    const NAME: &'static str = "WASM_LOAD";

    const EXECUTION_STATE: ExecutionState = ExecutionState::WASM_LOAD;

    fn configure(cb: &mut ConstrainBuilderCommon<F>) -> Self {

        let opcode_load_offset = cb.alloc_common_range_value();

        let load_start_block_index = cb.alloc_common_range_value();
        let load_start_block_inner_offset = cb.alloc_common_range_value();
        let load_start_block_inner_offset_helper = cb.alloc_common_range_value();

        let load_end_block_index = cb.alloc_common_range_value();
        let load_end_block_inner_offset = cb.alloc_common_range_value();
        let load_end_block_inner_offset_helper = cb.alloc_common_range_value();

        let load_value1 = cb.alloc_u64_on_u8();
        let load_value2 = cb.alloc_u64_on_u8();
        let offset_modulus = cb.alloc_u64_on_u8();
        let res = cb.alloc_u64();
        let value_in_heap = cb.alloc_u64();
        let load_base = cb.alloc_u64();

        let mask_bits = [0; 16].map(|_| cb.alloc_bit_value());
        let is_one_byte = cb.alloc_bit_value();
        let is_two_bytes = cb.alloc_bit_value();
        let is_four_bytes = cb.alloc_bit_value();
        let is_eight_bytes = cb.alloc_bit_value();
        let is_sign = cb.alloc_bit_value();
        let is_i64 = cb.alloc_bit_value();
        let vtype = cb.alloc_common_range_value();

        let highest_u4 = [0; 4].map(|_| cb.alloc_bit_value());

        //let lookup_offset_len_bits = common.alloc_offset_len_bits_table_lookup();
        //let lookup_pow = common.alloc_pow_table_lookup();

        let current_memory_page_size = cb.allocated_memory_pages_cell();
        let address_within_allocated_pages_helper = cb.alloc_common_range_value();

        cb.stack_pop(raw_address.expr());
        cb.stack_pop(block_value1.expr());
        cb.stack_pop(block_value2.expr());
        cb.stack_push(value.expr());

        cb.require_zeros("op_load: start end offset <= 7", vec![
            load_start_block_inner_offset.expr()
                + load_start_block_inner_offset_helper.expr()
                - 7.expr(),
            load_end_block_inner_offset.expr()
                + load_end_block_inner_offset_helper.expr()
                - 7.expr(),
        ]);

        cb.require_zeros("op_load: start end equation, start_index * 8 + start_offset + len = stop_index * 8 + stop_offset + 1", {
            let len = 1.expr()
                + is_two_bytes.expr() * 1.expr()
                + is_four_bytes.expr() * 3.expr()
                + is_eight_bytes.expr() * 7.expr();
            vec![
                load_start_block_index.expr() * 8.expr()
                    + load_start_block_inner_offset.expr()
                    + len
                    - 1.expr()
                    - load_end_block_index.expr() * 8.expr()
                    - load_end_block_inner_offset.expr(),
            ]
        });

        cb.require_zeros("op_load: start load_base", vec![
            load_base.expr() + opcode_load_offset.expr()
                - load_start_block_index.expr() * 8.expr()
                - load_start_block_inner_offset.expr(),
        ]);

        cb.require_zeros("op_load: length", vec![
            is_one_byte.expr()
                + is_two_bytes.expr()
                + is_four_bytes.expr()
                + is_eight_bytes.expr()
                - 1.expr(),
        ]);

        cb.require_zeros("op_load: mask_bits offset len", {
            let len = 1.expr()
                + is_two_bytes.expr() * 1.expr()
                + is_four_bytes.expr() * 3.expr()
                + is_eight_bytes.expr() * 7.expr();
            let (_, bits_encode) = mask_bits
                .map(|c| c.expr(meta))
                .into_iter()
                .enumerate()
                .reduce(|(_, acc), (i, e)| (i, acc + e * (1u64 << i).expr()))
                .unwrap();
            vec![
                lookup_offset_len_bits.expr()
                    - offset_len_bits_encode_expr(
                        load_start_block_inner_offset.expr(),
                        len,
                        bits_encode,
                    ),
            ]
        });

        cb.require_zeros("op_load: pow table lookup", vec![
            lookup_pow.expr(meta)
                - pow_table_encode(
                    offset_modulus.expr(),
                    load_start_block_inner_offset.expr() * 8.expr(),
                ),
        ]);

        /*constraint_builder.push(
            "op_load value_in_heap",
            Box::new(move |meta| {
                let mut acc = value_in_heap.expr(meta) * offset_modulus.expr(meta);
                for i in 0..8 {
                    acc = acc
                        - load_value1.u8_expr(meta, i)
                            * constant!(bn_to_field(&(BigUint::from(1u64) << (i * 8))))
                            * mask_bits[i as usize].expr(meta);
                    acc = acc
                        - load_value2.u8_expr(meta, i)
                            * constant!(bn_to_field(&(BigUint::from(1u64) << (i * 8 + 64))))
                            * mask_bits[i as usize + 8].expr(meta);
                }
                vec![acc]
            }),
        );*/

        /*constraint_builder.push(
            "op_load value: value = padding + value_in_heap",
            Box::new(move |meta| {
                let mut acc = is_one_byte.expr(meta) * value_in_heap.u4_expr(meta, 1)
                    + is_two_bytes.expr(meta) * value_in_heap.u4_expr(meta, 3)
                    + is_four_bytes.expr(meta) * value_in_heap.u4_expr(meta, 7)
                    + is_eight_bytes.expr(meta) * value_in_heap.u4_expr(meta, 15);
                for i in 0..4 {
                    acc = acc - highest_u4[i].expr(meta) * constant_from!(1u64 << 3 - i as u64)
                }
                let padding = is_one_byte.expr(meta) * constant_from!(0xffffff00)
                    + is_two_bytes.expr(meta) * constant_from!(0xffff0000)
                    + (constant_from!(1) - is_eight_bytes.expr(meta))
                        * is_i64.expr(meta)
                        * constant_from!(0xffffffff00000000);
                vec![
                    res.expr(meta)
                        - value_in_heap.expr(meta)
                        - highest_u4[0].expr(meta) * is_sign.expr(meta) * padding,
                    acc,
                ]
            }),
        );*/

        cb.require_zeros("op_load: is_i64 = 1 when vtype = 2", vec![
            is_i64.expr() + 1.expr() - vtype.expr()
        ]);

        cb.require_zeros("op_load: allocated address", {
            let len = 1.expr()
                + is_two_bytes.expr(meta) * 1.expr()
                + is_four_bytes.expr(meta) * 3.expr()
                + is_eight_bytes.expr(meta) * 7.expr();
            vec![
                load_base.expr()
                    + opcode_load_offset.expr()
                    + len
                    + address_within_allocated_pages_helper.expr()
                    - current_memory_page_size.expr() * WASM_PAGE_SIZE.expr(),
            ]
        });

        let opcode = cb.query_cell();

        // State transition
        let step_state_transition = StepStateTransition {
            rw_counter: Delta(2.expr()),
            program_counter: Delta(1.expr()),
            stack_pointer: Delta(0.expr()),
            // TODO: Change opcode.
            gas_left: Delta(-OpcodeId::I32Eqz.constant_gas_cost().expr()),
            ..StepStateTransition::default()
        };
        let same_context = SameContextGadget::construct(cb, opcode, step_state_transition);

        Self {
            same_context,
            opcode_load_offset,
            load_start_block_index,
            load_start_block_inner_offset,
            load_start_block_inner_offset_helper,
            load_end_block_index,
            load_end_block_inner_offset,
            load_end_block_inner_offset_helper,
            load_value1,
            load_value2,
            mask_bits,
            offset_modulus,
            load_base,
            res,
            value_in_heap,
            is_one_byte,
            is_two_bytes,
            is_four_bytes,
            is_eight_bytes,
            is_sign,
            is_i64,
            highest_u4,
            vtype,
            //lookup_stack_write,
            //lookup_offset_len_bits,
            lookup_pow,
            address_within_allocated_pages_helper,
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

        let opcode = step.opcode.unwrap();

        let [rhs, lhs, value] = [step.rw_indices[0], step.rw_indices[1], step.rw_indices[2]]
            .map(|idx| block.rws[idx].stack_value());

/*
        self.value.assign(region, offset, Value::known(value.to_scalar().unwrap()))?;
        self.value_inv.assign(region, offset, Value::known(F::from(value.as_u64()).invert().unwrap_or(F::zero())))?;
        self.res.assign(region, offset, Value::known(res.to_scalar().unwrap()))?;

        match opcode {
            OpcodeId::I64Eqz => {
                let zero_or_one = (value.as_u64() == 0) as u64;
                self.res.assign(region, offset, Value::known(F::from(zero_or_one)))?;
            }
            OpcodeId::I32Eqz => {
                let zero_or_one = (value.as_u32() == 0) as u64;
                self.res.assign(region, offset, Value::known(F::from(zero_or_one)))?;
            }
            _ => unreachable!("not supported opcode: {:?}", opcode),
        };
 
        let is_i64 = matches!(opcode,
            OpcodeId::I64Eqz
        );
        self.is_i64.assign(region, offset, Value::known(F::from(is_i64 as u64)))?;
*/

        Ok(())
    }
}

#[cfg(test)]
mod test {
    use eth_types::{bytecode, Bytecode};
    use mock::TestContext;

    use crate::test_util::CircuitTestBuilder;

    fn run_test(bytecode: Bytecode) {
        CircuitTestBuilder::new_from_test_ctx(
            TestContext::<2, 1>::simple_ctx_with_bytecode(bytecode).unwrap(),
        ).run()
    }

/*
    #[test]
    fn test_i32_eqz() {
        run_test(bytecode! {
            I32Const[0]
            I32Eqz
            Drop
            I32Const[1]
            I32Eqz
            Drop
        });
    }

    #[test]
    fn test_i64_eqz() {
        run_test(bytecode! {
            I64Const[0]
            I64Eqz
            Drop
            I64Const[1]
            I64Eqz
            Drop
        });
    }
*/
}
