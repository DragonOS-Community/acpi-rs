use crate::{
    name_object::{name_string, super_name, target},
    opcode::{self, opcode},
    parser::{
        choice,
        comment_scope,
        id,
        make_parser_concrete,
        take,
        take_to_end_of_pkglength,
        try_with_context,
        Parser,
    },
    pkg_length::pkg_length,
    term_object::{data_ref_object, term_arg},
    value::AmlValue,
    AmlError,
    DebugVerbosity,
};
use alloc::vec::Vec;
use core::convert::TryInto;

/// Type 2 opcodes return a value and so can be used in expressions.
pub fn type2_opcode<'a, 'c>() -> impl Parser<'a, 'c, AmlValue>
where
    'c: 'a,
{
    /*
     * Type2Opcode := DefAquire | DefAdd | DefAnd | DefBuffer | DefConcat | DefConcatRes |
     *                DefCondRefOf | DefCopyObject | DefDecrement | DefDerefOf | DefDivide |
     *                DefFindSetLeftBit | DefFindSetRightBit | DefFromBCD | DefIncrement | DefIndex |
     *                DefLAnd | DefLEqual | DefLGreater | DefLGreaterEqual | DefLLess | DefLLessEqual |
     *                DefMid | DefLNot | DefLNotEqual | DefLoadTable | DefLOr | DefMatch | DefMod |
     *                DefMultiply | DefNAnd | DefNOr | DefNot | DefObjectType | DefOr | DefPackage |
     *                DefVarPackage | DefRefOf | DefShiftLeft | DefShitRight | DefSizeOf | DefStore |
     *                DefSubtract | DefTimer | DefToBCD | DefToBuffer | DefToDecimalString |
     *                DefToHexString | DefToInteger | DefToString | DefWait | DefXOr | MethodInvocation
     *
     * NOTE: MethodInvocation should always appear last in the choice.
     */
    make_parser_concrete!(comment_scope(
        DebugVerbosity::AllScopes,
        "Type2Opcode",
        choice!(
            def_buffer(),
            def_l_equal(),
            def_package(),
            def_shift_left(),
            def_shift_right(),
            def_store(),
            method_invocation()
        ),
    ))
}

pub fn def_buffer<'a, 'c>() -> impl Parser<'a, 'c, AmlValue>
where
    'c: 'a,
{
    /*
     * DefBuffer := 0x11 PkgLength BufferSize ByteList
     * BufferSize := TermArg => Integer
     *
     * XXX: The spec says that zero-length buffers (e.g. the PkgLength is 0) are illegal, but
     * we've encountered them in QEMU-generated tables, so we return an empty buffer in these
     * cases.
     */
    opcode(opcode::DEF_BUFFER_OP)
        .then(comment_scope(
            DebugVerbosity::AllScopes,
            "DefBuffer",
            pkg_length().then(term_arg()).feed(|(pkg_length, buffer_size)| {
                take_to_end_of_pkglength(pkg_length).map_with_context(move |bytes, context| {
                    let length = try_with_context!(context, buffer_size.as_integer(context));
                    (Ok((bytes.to_vec(), length)), context)
                })
            }),
        ))
        .map(|((), (bytes, buffer_size))| Ok(AmlValue::Buffer { bytes, size: buffer_size }))
}

fn def_l_equal<'a, 'c>() -> impl Parser<'a, 'c, AmlValue>
where
    'c: 'a,
{
    /*
     * DefLEqual := 0x93 Operand Operand
     * Operand := TermArg => Integer
     */
    opcode(opcode::DEF_L_EQUAL_OP)
        .then(comment_scope(
            DebugVerbosity::AllScopes,
            "DefLEqual",
            term_arg().then(term_arg()).map_with_context(|(left_arg, right_arg), context| {
                let left = try_with_context!(context, left_arg.as_integer(context));
                let right = try_with_context!(context, right_arg.as_integer(context));
                (Ok(AmlValue::Boolean(left == right)), context)
            }),
        ))
        .map(|((), result)| Ok(result))
}

pub fn def_package<'a, 'c>() -> impl Parser<'a, 'c, AmlValue>
where
    'c: 'a,
{
    /*
     * DefPackage := 0x12 PkgLength NumElements PackageElementList
     * NumElements := ByteData
     * PackageElementList := Nothing | <PackageElement PackageElementList>
     * PackageElement := DataRefObject | NameString
     */
    opcode(opcode::DEF_PACKAGE_OP)
        .then(comment_scope(
            DebugVerbosity::AllScopes,
            "DefPackage",
            pkg_length().then(take()).feed(|(pkg_length, num_elements)| {
                move |mut input, mut context| {
                    let mut package_contents = Vec::new();

                    while pkg_length.still_parsing(input) {
                        let (new_input, new_context, value) = package_element().parse(input, context)?;
                        input = new_input;
                        context = new_context;

                        package_contents.push(value);
                    }

                    assert_eq!(package_contents.len(), num_elements as usize);
                    Ok((input, context, AmlValue::Package(package_contents)))
                }
            }),
        ))
        .map(|((), package)| Ok(package))
}

pub fn package_element<'a, 'c>() -> impl Parser<'a, 'c, AmlValue>
where
    'c: 'a,
{
    choice!(data_ref_object(), name_string().map(|string| Ok(AmlValue::String(string.as_string()))))
}

fn def_shift_left<'a, 'c>() -> impl Parser<'a, 'c, AmlValue>
where
    'c: 'a,
{
    /*
     * DefShiftLeft := 0x79 Operand ShiftCount Target
     * Operand := TermArg => Integer
     * ShiftCount := TermArg => Integer
     */
    opcode(opcode::DEF_SHIFT_LEFT)
        .then(comment_scope(DebugVerbosity::Scopes, "DefShiftLeft", term_arg().then(term_arg()).then(target())))
        .map_with_context(|((), ((operand, shift_count), target)), context| {
            let operand = try_with_context!(context, operand.as_integer(context));
            let shift_count = try_with_context!(context, shift_count.as_integer(context));
            let shift_count =
                try_with_context!(context, shift_count.try_into().map_err(|_| AmlError::InvalidShiftLeft));

            let result = AmlValue::Integer(try_with_context!(
                context,
                operand.checked_shl(shift_count).ok_or(AmlError::InvalidShiftLeft)
            ));

            try_with_context!(context, context.store(target, result.clone()));
            (Ok(result), context)
        })
}

fn def_shift_right<'a, 'c>() -> impl Parser<'a, 'c, AmlValue>
where
    'c: 'a,
{
    /*
     * DefShiftRight := 0x7a Operand ShiftCount Target
     * Operand := TermArg => Integer
     * ShiftCount := TermArg => Integer
     */
    opcode(opcode::DEF_SHIFT_RIGHT)
        .then(comment_scope(DebugVerbosity::Scopes, "DefShiftRight", term_arg().then(term_arg()).then(target())))
        .map_with_context(|((), ((operand, shift_count), target)), context| {
            let operand = try_with_context!(context, operand.as_integer(context));
            let shift_count = try_with_context!(context, shift_count.as_integer(context));
            let shift_count =
                try_with_context!(context, shift_count.try_into().map_err(|_| AmlError::InvalidShiftRight));

            let result = AmlValue::Integer(try_with_context!(
                context,
                operand.checked_shr(shift_count).ok_or(AmlError::InvalidShiftRight)
            ));

            try_with_context!(context, context.store(target, result.clone()));
            (Ok(result), context)
        })
}

fn def_store<'a, 'c>() -> impl Parser<'a, 'c, AmlValue>
where
    'c: 'a,
{
    /*
     * DefStore := 0x70 TermArg SuperName
     *
     * Implicit conversion is only applied when the destination target is a `Name` - not when we
     * are storing into a method local or argument (these stores are semantically identical to
     * CopyObject). We must also make sure to return a copy of the data that is in the destination
     * after the store (as opposed to the data we think we put into it), because some stores can
     * alter the data during the store.
     */
    opcode(opcode::DEF_STORE_OP)
        .then(comment_scope(DebugVerbosity::Scopes, "DefStore", term_arg().then(super_name())))
        .map_with_context(|((), (value, target)), context| {
            (Ok(try_with_context!(context, context.store(target, value))), context)
        })
}

fn method_invocation<'a, 'c>() -> impl Parser<'a, 'c, AmlValue>
where
    'c: 'a,
{
    /*
     * MethodInvocation := NameString TermArgList
     *
     * MethodInvocation is the worst of the AML structures, because you're meant to figure out how much you're
     * meant to parse using the name of the method (by knowing from its definition how how many arguments it
     * takes). However, the definition of a method can in theory appear after an invocation of that method, and
     * so parsing them properly can be very difficult.
     * NOTE: We don't support the case of the definition appearing after the invocation.
     */
    comment_scope(
        DebugVerbosity::Scopes,
        "MethodInvocation",
        name_string()
            .map_with_context(move |name, context| {
                let (_, handle) =
                    try_with_context!(context, context.namespace.search(&name, &context.current_scope)).clone();
                (Ok(handle), context)
            })
            .feed(|handle| {
                id().map_with_context(move |(), context| {
                    let object = try_with_context!(context, context.namespace.get(handle));
                    if let AmlValue::Method { ref code, .. } = object {
                        // TODO: we need to allow a method to be invoked from inside another method before we can
                        // implement this (basically a stack of contexts) then implement this
                        unimplemented!()
                    } else {
                        // We appear to be seeing AML where a MethodInvocation actually doesn't point to a method
                        // at all, which isn't mentioned in the spec afaict.  However, if we treat it as an
                        // "invocation" with 0 arguments and simply return the object, the AML seems to do sensible
                        // things.
                        (Ok(object.clone()), context)
                    }
                })
            }),
    )
}
