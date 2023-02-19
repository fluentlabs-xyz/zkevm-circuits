use self::op_configure::EventTableOpcodeConfig;
use super::*;
use crate::wasm_circuit::circuits::{
    etable_compact::op_configure::{
        op_bin::BinConfigBuilder, op_bin_bit::BinBitConfigBuilder,
        op_bin_shift::BinShiftConfigBuilder, op_br::BrConfigBuilder, op_br_if::BrIfConfigBuilder,
        op_br_if_eqz::BrIfEqzConfigBuilder, op_br_table::BrTableConfigBuilder,
        op_call::CallConfigBuilder, op_call_indirect::CallIndirectConfigBuilder,
        op_const::ConstConfigBuilder, op_conversion::ConversionConfigBuilder,
        op_drop::DropConfigBuilder, op_global_get::GlobalGetConfigBuilder,
        op_global_set::GlobalSetConfigBuilder, op_load::LoadConfigBuilder,
        op_local_get::LocalGetConfigBuilder, op_local_set::LocalSetConfigBuilder,
        op_local_tee::LocalTeeConfigBuilder, op_memory_grow::MemoryGrowConfigBuilder,
        op_memory_size::MemorySizeConfigBuilder, op_rel::RelConfigBuilder,
        op_return::ReturnConfigBuilder, op_select::SelectConfigBuilder,
        op_store::StoreConfigBuilder, op_test::TestConfigBuilder, op_unary::UnaryConfigBuilder,
        ConstraintBuilder, EventTableCellAllocator, EventTableOpcodeConfigBuilder,
    },
    itable::{encode_inst_expr, Encode},
    traits::ConfigureLookupTable,
    utils::bn_to_field,
};
use halo2_proofs::{
    circuit::Cell,
    plonk::{Advice, Column, ConstraintSystem, Error, Expression, Fixed, VirtualCells},
};
use crate::wasm_circuit::specs::{
    configure_table::ConfigureTable,
    etable::{EventTable, EventTableEntry},
    itable::OpcodeClass,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    marker::PhantomData,
    rc::Rc,
};
use eth_types::Field;
use crate::{constant_from, curr, fixed_curr, nextn};

pub mod assign;
pub mod expression;
pub mod op_configure;

// TODO:
// 1. add constraints for termination
// 2. add input output for circuits

const ETABLE_STEP_SIZE: usize = 20usize;
const U4_COLUMNS: usize = 3usize;
const U8_COLUMNS: usize = 2usize;
const BITS_COLUMNS: usize = 2usize;
const MTABLE_LOOKUPS_SIZE: usize = 6usize;
const MAX_OP_LVL1: i32 = (ETABLE_STEP_SIZE >> 1) as i32;
const MAX_OP_LVL2: i32 = ETABLE_STEP_SIZE as i32;

fn opclass_to_two_level(class: OpcodeClassPlain) -> (usize, usize) {
    let mut id = class.0 as i32;
    assert!(id <= MAX_OP_LVL1 * (MAX_OP_LVL2 - MAX_OP_LVL1));

    id -= 1;

    (
        (id / MAX_OP_LVL1) as usize,
        ((id % MAX_OP_LVL1) + MAX_OP_LVL1) as usize,
    )
}

pub(crate) enum EventTableBitColumnRotation {
    Enable = 0,
    Max,
}

pub(crate) enum EventTableCommonRangeColumnRotation {
    RestMOps = 0,
    RestJOps,
    InputIndex,
    EID,
    MOID,
    FID,
    IID,
    MMID,
    SP,
    LastJumpEid,
    AllocatedMemoryPages,
    Max,
}

pub(crate) enum EventTableUnlimitColumnRotation {
    ITableLookup = 0,
    BrTableLookup = 1,
    JTableLookup = 2,
    PowTableLookup = 3,
    OffsetLenBitsTableLookup = 4,
    MTableLookupStart = 5,
    U64Start = 6 + MTABLE_LOOKUPS_SIZE as isize,
}

pub enum MLookupItem {
    First = 0,
    Second,
    Third,
    Fourth,
    Fifth,
    Six,
}

impl From<usize> for MLookupItem {
    fn from(i: usize) -> Self {
        match i {
            0 => Self::First,
            1 => Self::Second,
            2 => Self::Third,
            3 => Self::Fourth,
            4 => Self::Fifth,
            5 => Self::Six,
            _ => unreachable!(),
        }
    }
}

#[derive(Clone)]
pub struct Status {
    pub eid: u64, // serial event id
    pub moid: u16, // module id
    pub fid: u16, // function id
    pub iid: u16, // offset of the instruction in a particular function
    pub mmid: u16, // memory block instance id
    pub sp: u64, // stack pointer
    pub last_jump_eid: u64,
    pub allocated_memory_pages: u16,
}

pub struct StepStatus<'a> {
    pub current: &'a Status,
    pub next: &'a Status,
    pub configure: ConfigureTable,
}

impl TryFrom<u32> for MLookupItem {
    type Error = Error;

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::First),
            1 => Ok(Self::Second),
            2 => Ok(Self::Third),
            3 => Ok(Self::Fourth),
            _ => unreachable!(),
        }
    }
}

#[derive(Clone)]
pub struct ConstantCell(pub Column<Fixed>);

#[derive(Clone)]
pub struct EventTableCommonConfig<F> {
    pub sel: Column<Fixed>,
    pub block_first_line_sel: Column<Fixed>,

    pub shared_bits: [Column<Advice>; BITS_COLUMNS],
    pub opcode_bits: Column<Advice>,

    pub state: Column<Advice>,

    pub unlimited: Column<Advice>,

    pub itable_lookup: Column<Fixed>,
    pub brtable_lookup: Column<Fixed>,
    pub jtable_lookup: Column<Fixed>,
    pub mtable_lookup: Column<Fixed>,
    pub pow_table_lookup: Column<Fixed>,
    pub offset_len_bits_table_lookup: Column<Fixed>,

    pub aux: Column<Advice>,

    pub u4_bop: Column<Advice>,
    pub u4_shared: [Column<Advice>; U4_COLUMNS],
    pub u8_shared: [Column<Advice>; U8_COLUMNS],

    pub circuit_configure: CircuitConfigure,

    _mark: PhantomData<F>,
}

#[derive(Clone)]
pub struct EventTableConfig<F: Field> {
    common_config: EventTableCommonConfig<F>,
    op_configs: BTreeMap<OpcodeClassPlain, Rc<Box<dyn EventTableOpcodeConfig<F>>>>,
}

impl<F: Field> EventTableConfig<F> {

    #[allow(unused_variables)]
    pub fn configure(
        meta: &mut ConstraintSystem<F>,
        cols: &mut (impl Iterator<Item = Column<Advice>> + Clone),
        circuit_configure: &CircuitConfigure,
        rtable: &RangeTableConfig<F>,
        itable: &InstructionTableConfig<F>,
        mtable: &MemoryTableConfig<F>,
        jtable: &JumpTableConfig<F>,
        brtable: &BrTableConfig<F>,
        opcode_set: &BTreeSet<OpcodeClassPlain>,
    ) -> Self {
        let sel = meta.fixed_column();
        let block_first_line_sel = meta.fixed_column();
        let shared_bits = [0; BITS_COLUMNS].map(|_| cols.next().unwrap());
        let opcode_bits = cols.next().unwrap();

        let state = cols.next().unwrap();
        let aux = cols.next().unwrap();
        let unlimited = cols.next().unwrap();

        let itable_lookup = meta.fixed_column();
        let brtable_lookup = meta.fixed_column();
        let jtable_lookup = meta.fixed_column();
        let mtable_lookup = meta.fixed_column();
        let pow_table_lookup = meta.fixed_column();
        let offset_len_bits_table_lookup = meta.fixed_column();

        let u4_shared = [0; U4_COLUMNS].map(|_| cols.next().unwrap());
        let u8_shared = [0; U8_COLUMNS].map(|_| cols.next().unwrap());
        let u4_bop = cols.next().unwrap();

        meta.enable_equality(state);
        meta.create_gate("etable opcode bits", |meta| {
            vec![curr!(meta, opcode_bits) * (curr!(meta, opcode_bits) - constant_from!(1))]
                .into_iter()
                .map(|x| x * fixed_curr!(meta, sel))
                .collect::<Vec<_>>()
        });

        meta.create_gate("etable shared bits", |meta| {
            shared_bits
                .iter()
                .map(|x| {
                    curr!(meta, *x) * (curr!(meta, *x) - constant_from!(1)) * fixed_curr!(meta, sel)
                })
                .collect::<Vec<_>>()
        });

        rtable.configure_in_u4_bop_set(meta, "etable u4 bop", |meta| {
            curr!(meta, u4_bop) * fixed_curr!(meta, sel)
        });

        rtable.configure_in_u4_bop_calc_set(meta, "etable u4 bop calc", |meta| {
            (
                curr!(meta, u4_shared[0]),
                curr!(meta, u4_shared[1]),
                curr!(meta, u4_shared[2]),
                curr!(meta, u4_bop) * fixed_curr!(meta, sel),
            )
        });

        rtable.configure_in_common_range(meta, "etable aux in common", |meta| {
            curr!(meta, state) * fixed_curr!(meta, sel)
        });

        for i in 0..U4_COLUMNS {
            rtable.configure_in_u4_range(meta, "etable u4", |meta| {
                curr!(meta, u4_shared[i]) * fixed_curr!(meta, sel)
            });
        }

        for i in 0..U8_COLUMNS {
            rtable.configure_in_u8_range(meta, "etable u8", |meta| {
                curr!(meta, u8_shared[i]) * fixed_curr!(meta, sel)
            });
        }

        itable.configure_in_table(meta, "etable itable lookup", |meta| {
            curr!(meta, aux) * fixed_curr!(meta, itable_lookup)
        });

        brtable.configure_in_table(meta, "etable brtable lookup", |meta| {
            curr!(meta, aux) * fixed_curr!(meta, brtable_lookup)
        });

        mtable.configure_in_table(meta, "etable mtable lookup", |meta| {
            curr!(meta, aux) * fixed_curr!(meta, mtable_lookup)
        });

        jtable.configure_in_table(meta, "etable jtable lookup", |meta| {
            curr!(meta, aux) * fixed_curr!(meta, jtable_lookup)
        });

        rtable.configure_in_pow_set(meta, "etable pow_table lookup", |meta| {
            curr!(meta, aux) * fixed_curr!(meta, pow_table_lookup)
        });

        rtable.configure_in_offset_len_bits_set(meta, "etable offset len bits lookup", |meta| {
            curr!(meta, aux) * fixed_curr!(meta, offset_len_bits_table_lookup)
        });

        for i in 0..U4_COLUMNS {
            meta.create_gate("etable u64 on u4", |meta| {
                let mut acc = nextn!(
                    meta,
                    aux,
                    EventTableUnlimitColumnRotation::U64Start as i32 + i as i32
                );
                let mut base = 1u64;
                for j in 0..16 {
                    acc = acc - nextn!(meta, u4_shared[i], j) * constant_from!(base);
                    base <<= 4;
                }

                vec![acc * fixed_curr!(meta, block_first_line_sel)]
            });
        }

        for i in 0..U8_COLUMNS {
            meta.create_gate("etable u64 on u8", |meta| {
                let mut acc1 = nextn!(
                    meta,
                    aux,
                    EventTableUnlimitColumnRotation::U64Start as i32
                        + U4_COLUMNS as i32
                        + i as i32 * 2
                );
                let mut base = 1u64;
                for j in 0..8 {
                    acc1 = acc1 - nextn!(meta, u8_shared[i], j) * constant_from!(base);
                    base <<= 8;
                }

                let mut acc2 = nextn!(
                    meta,
                    aux,
                    EventTableUnlimitColumnRotation::U64Start as i32
                        + U4_COLUMNS as i32
                        + i as i32 * 2
                        + 1
                );
                let mut base = 1u64;
                for j in 8..16 {
                    acc2 = acc2 - nextn!(meta, u8_shared[i], j) * constant_from!(base);
                    base <<= 8;
                }

                vec![
                    acc1 * fixed_curr!(meta, block_first_line_sel),
                    acc2 * fixed_curr!(meta, block_first_line_sel),
                ]
            });
        }

        let common_config = EventTableCommonConfig {
            sel,
            block_first_line_sel,
            shared_bits,
            opcode_bits,
            state,
            unlimited,
            itable_lookup,
            brtable_lookup,
            jtable_lookup,
            mtable_lookup,
            pow_table_lookup,
            offset_len_bits_table_lookup,
            aux,
            u4_shared,
            u8_shared,
            u4_bop,
            circuit_configure: circuit_configure.clone(),
            _mark: PhantomData,
        };

        let mut op_bitmaps: BTreeMap<OpcodeClassPlain, (i32, i32)> = BTreeMap::new();
        let mut op_configs: BTreeMap<OpcodeClassPlain, Rc<Box<dyn EventTableOpcodeConfig<F>>>> =
            BTreeMap::new();

        macro_rules! configure [
            ($op:expr, $x:ident) => ({
                let op = OpcodeClassPlain($op as usize);
                if opcode_set.contains(&op) {
                    let (op_lvl1, op_lvl2) = opclass_to_two_level(op);
                    let mut allocator = EventTableCellAllocator::new(&common_config);
                    let mut constraint_builder = ConstraintBuilder::new(meta);

                    let config = $x::configure(
                        &mut allocator,
                        &mut constraint_builder,
                    );

                    constraint_builder.finalize(|meta|
                        fixed_curr!(meta, common_config.block_first_line_sel) *
                            common_config.op_enabled(meta, op_lvl1 as i32, op_lvl2 as i32)
                    );

                    op_bitmaps.insert(op, (op_lvl1 as i32, op_lvl2 as i32));
                    op_configs.insert(op, Rc::new(config));
                }
    })
        ];

        macro_rules! configure_foreign [
            ($op:expr, $x:ident, $call_info:ident) => ({
                let op = OpcodeClassPlain(OpcodeClass::ForeignPluginStart as usize + $op as usize);

                if opcode_set.contains(&op) {
                    let (op_lvl1, op_lvl2) = opclass_to_two_level(op);
                    let mut allocator = EventTableCellAllocator::new(&common_config);
                    let mut constraint_builder = ConstraintBuilder::new(meta);

                    let config = $x::configure(
                        &mut allocator,
                        &mut constraint_builder,
                        &$call_info{},
                    );

                    constraint_builder.finalize(foreign_tables, |meta|
                        fixed_curr!(meta, common_config.block_first_line_sel) *
                            common_config.op_enabled(meta, op_lvl1 as i32, op_lvl2 as i32)
                    );

                    op_bitmaps.insert(op, (op_lvl1 as i32, op_lvl2 as i32));
                    op_configs.insert(op, Rc::new(config));
                }
            })
        ];

        configure!(OpcodeClass::Return, ReturnConfigBuilder);
        configure!(OpcodeClass::Br, BrConfigBuilder);
        configure!(OpcodeClass::BrIfEqz, BrIfEqzConfigBuilder);
        configure!(OpcodeClass::BrTable, BrTableConfigBuilder);
        configure!(OpcodeClass::Call, CallConfigBuilder);
        configure!(OpcodeClass::CallIndirect, CallIndirectConfigBuilder);
        configure!(OpcodeClass::Const, ConstConfigBuilder);
        configure!(OpcodeClass::Drop, DropConfigBuilder);
        configure!(OpcodeClass::LocalGet, LocalGetConfigBuilder);
        configure!(OpcodeClass::LocalSet, LocalSetConfigBuilder);
        configure!(OpcodeClass::LocalTee, LocalTeeConfigBuilder);
        configure!(OpcodeClass::GlobalGet, GlobalGetConfigBuilder);
        configure!(OpcodeClass::GlobalSet, GlobalSetConfigBuilder);
        configure!(OpcodeClass::Bin, BinConfigBuilder);
        configure!(OpcodeClass::BinBit, BinBitConfigBuilder);
        configure!(OpcodeClass::BinShift, BinShiftConfigBuilder);
        configure!(OpcodeClass::Unary, UnaryConfigBuilder);
        configure!(OpcodeClass::BrIf, BrIfConfigBuilder);
        configure!(OpcodeClass::Load, LoadConfigBuilder);
        configure!(OpcodeClass::Store, StoreConfigBuilder);
        configure!(OpcodeClass::MemorySize, MemorySizeConfigBuilder);
        configure!(OpcodeClass::MemoryGrow, MemoryGrowConfigBuilder);
        configure!(OpcodeClass::Rel, RelConfigBuilder);
        configure!(OpcodeClass::Select, SelectConfigBuilder);
        configure!(OpcodeClass::Test, TestConfigBuilder);
        configure!(OpcodeClass::Conversion, ConversionConfigBuilder);

        meta.create_gate("enable seq", |meta| {
            vec![
                common_config.next_enable(meta)
                    * (common_config.enable(meta) - constant_from!(1))
                    * fixed_curr!(meta, common_config.block_first_line_sel),
            ]
        });

        meta.create_gate("etable common change", |meta| {
            let mut rest_mops_acc =
                common_config.next_rest_mops(meta) - common_config.rest_mops(meta);
            let mut rest_jops_acc =
                common_config.next_rest_jops(meta) - common_config.rest_jops(meta);
            let mut input_index_acc =
                common_config.input_index(meta) - common_config.next_input_index(meta);
            let mut moid_acc = common_config.next_moid(meta) - common_config.moid(meta);
            let mut fid_acc = common_config.next_fid(meta) - common_config.fid(meta);
            let mut iid_acc =
                common_config.next_iid(meta) - common_config.iid(meta) - constant_from!(1);
            let mut sp_acc = common_config.next_sp(meta) - common_config.sp(meta);
            let mut last_jump_eid_acc =
                common_config.next_last_jump_eid(meta) - common_config.last_jump_eid(meta);
            let mut allocated_memory_pages_acc = common_config.next_allocated_memory_pages(meta)
                - common_config.allocated_memory_pages(meta);

            let eid_diff =
                common_config.next_eid(meta) - common_config.eid(meta) - constant_from!(1);
            // MMID equals to MOID in single module version
            let mmid_diff = common_config.mmid(meta) - common_config.moid(meta);

            let mut itable_lookup = common_config.itable_lookup(meta);
            let mut brtable_lookup = common_config.brtable_lookup(meta);
            let mut jtable_lookup = common_config.jtable_lookup(meta);
            let mut mtable_lookup = vec![];

            for i in 0..MTABLE_LOOKUPS_SIZE {
                mtable_lookup.push(common_config.mtable_lookup(meta, i as i32));
            }

            for (op, (lvl1, lvl2)) in op_bitmaps.iter() {
                let config = op_configs.get(op).unwrap();
                match config.mops(meta) {
                    Some(e) => {
                        rest_mops_acc =
                            rest_mops_acc + e * common_config.op_enabled(meta, *lvl1, *lvl2)
                    }
                    _ => {}
                }

                match config.jops(meta) {
                    Some(e) => {
                        rest_jops_acc = rest_jops_acc
                            + e * common_config.op_enabled(meta, *lvl1, *lvl2)
                                * common_config.next_enable(meta) // The last return is not accounting into.
                    }
                    _ => {}
                }

                match config.next_last_jump_eid(meta, &common_config) {
                    Some(e) => {
                        last_jump_eid_acc = last_jump_eid_acc
                            - (e - common_config.last_jump_eid(meta))
                                * common_config.op_enabled(meta, *lvl1, *lvl2)
                    }
                    _ => {}
                }

                match config.next_moid(meta, &common_config) {
                    Some(e) => {
                        moid_acc = moid_acc
                            - (e - common_config.moid(meta))
                                * common_config.op_enabled(meta, *lvl1, *lvl2)
                    }
                    _ => {}
                }

                match config.next_fid(meta, &common_config) {
                    Some(e) => {
                        fid_acc = fid_acc
                            - (e - common_config.fid(meta))
                                * common_config.op_enabled(meta, *lvl1, *lvl2)
                    }
                    _ => {}
                }

                match config.next_iid(meta, &common_config) {
                    Some(e) => {
                        iid_acc = iid_acc
                            + (constant_from!(1) + common_config.iid(meta) - e)
                                * common_config.op_enabled(meta, *lvl1, *lvl2)
                    }
                    _ => {}
                }

                match config.sp_diff(meta) {
                    Some(e) => sp_acc = sp_acc - e * common_config.op_enabled(meta, *lvl1, *lvl2),
                    _ => {}
                }

                match config.allocated_memory_pages_diff(meta) {
                    Some(e) => {
                        allocated_memory_pages_acc = allocated_memory_pages_acc
                            - e * common_config.op_enabled(meta, *lvl1, *lvl2)
                    }
                    _ => {}
                }

                itable_lookup = itable_lookup
                    - encode_inst_expr(
                        common_config.moid(meta),
                        common_config.mmid(meta),
                        common_config.fid(meta),
                        common_config.iid(meta),
                        config.opcode(meta),
                    ) * common_config.op_enabled(meta, *lvl1, *lvl2);

                match config.brtable_lookup(meta, &common_config) {
                    Some(e) => {
                        brtable_lookup =
                            brtable_lookup - e * common_config.op_enabled(meta, *lvl1, *lvl2)
                    }
                    _ => {}
                }

                match config.jtable_lookup(meta, &common_config) {
                    Some(e) => {
                        jtable_lookup =
                            jtable_lookup - e * common_config.op_enabled(meta, *lvl1, *lvl2)
                    }
                    _ => {}
                }

                match config.input_index_increase(meta, &common_config) {
                    Some(e) => {
                        input_index_acc =
                            input_index_acc + common_config.op_enabled(meta, *lvl1, *lvl2) * e
                    }
                    _ => {}
                }

                for i in 0..MTABLE_LOOKUPS_SIZE {
                    match config.mtable_lookup(meta, i.try_into().unwrap(), &common_config) {
                        Some(e) => {
                            mtable_lookup[i] = mtable_lookup[i].clone()
                                - e * common_config.op_enabled(meta, *lvl1, *lvl2)
                        }
                        _ => {}
                    }
                }
            }

            // TODO: elegantly handle the last row and then
            // delete common_config.next_enable(meta)
            vec![
                vec![
                    rest_mops_acc,
                    rest_jops_acc,
                    eid_diff * common_config.next_enable(meta),
                    moid_acc,
                    fid_acc,
                    iid_acc * common_config.next_enable(meta),
                    mmid_diff,
                    sp_acc * common_config.next_enable(meta),
                    last_jump_eid_acc,
                    allocated_memory_pages_acc * common_config.next_enable(meta),
                    itable_lookup,
                    brtable_lookup,
                    jtable_lookup,
                    input_index_acc * common_config.next_enable(meta),
                ],
                mtable_lookup,
            ]
            .into_iter()
            .flatten()
            .map(|x| x * common_config.enabled_block(meta))
            .collect::<Vec<_>>()
        });

        meta.create_gate("etable op lvl bits sum", |meta| {
            let mut acc_lvl1 = constant_from!(1);
            let mut acc_lvl2 = constant_from!(1);

            for i in 0..MAX_OP_LVL1 {
                acc_lvl1 = acc_lvl1 - nextn!(meta, common_config.opcode_bits, i);
            }

            for i in MAX_OP_LVL1..ETABLE_STEP_SIZE as i32 {
                acc_lvl2 = acc_lvl2 - nextn!(meta, common_config.opcode_bits, i);
            }

            vec![acc_lvl1, acc_lvl2]
                .into_iter()
                .map(|x| x * common_config.enabled_block(meta))
                .collect::<Vec<_>>()
        });

        Self {
            common_config,
            op_configs,
        }
    }
}

pub struct EventTableChip<F: Field> {
    config: EventTableConfig<F>,
}

impl<F: Field> EventTableChip<F> {
    pub fn new(config: EventTableConfig<F>) -> Self {
        EventTableChip { config }
    }

    pub(crate) fn assign(
        &self,
        ctx: &mut Context<'_, F>,
        etable: &EventTable,
        configure: ConfigureTable,
    ) -> Result<(Option<Cell>, Option<Cell>), Error> {
        self.config
            .common_config
            .assign(ctx, &self.config.op_configs, etable, configure)
    }
}
