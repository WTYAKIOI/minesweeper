# Apache 2.0 licensed

"""
This module provides functions for evaluating Python code,
which is intended to be secure.

Warning: Since this module is new, its possible there are flaws in it.


How it Works
============

- Restrict built-in namespace access.
- Restrict byte-codes used.


What Works
==========

This works:

    a ** cos(b / c) - sin(d)

This will fail:

    a.b

    __import__("os").unlink("path")
"""

__all__ = (
    "safe_eval",
    "raise_if_code_unsafe",
    )

import dis
import sys
import math


builtins_whitelist = {
    # basic funcs
    "all",
    "any",
    "len",
    "max",
    "min",

    # types
    "bool",
    "float",
    "int",

    # math
    "abs",
    "divmod",
    "pow",
    "round",
    "sum",
}

if sys.version_info[0:2] == (3, 11):
    opcode_whitelist = {
        # Shared with Python 3.10.
        'POP_TOP',
        'NOP',
        'UNARY_POSITIVE',
        'UNARY_NEGATIVE',
        'UNARY_NOT',
        'UNARY_INVERT',
        'BINARY_SUBSCR',
        'RETURN_VALUE',
        'BUILD_TUPLE',
        'BUILD_LIST',
        'BUILD_SET',
        'BUILD_MAP',
        'COMPARE_OP',
        'JUMP_FORWARD',
        'JUMP_IF_FALSE_OR_POP',
        'JUMP_IF_TRUE_OR_POP',
        'LOAD_GLOBAL',
        'IS_OP',
        'LOAD_FAST',
        'STORE_FAST',
        'DELETE_FAST',
        'BUILD_SLICE',
        'LOAD_DEREF',
        'STORE_DEREF',
        'DELETE_DEREF',
        'LOAD_CONST',
        'LOAD_NAME',
        'CALL_FUNCTION_EX',

        # New in Python 3.11.
        'BINARY_OP',
        'RESUME',
        'CACHE',
        'PUSH_NULL',
        'CALL',
        'PRECALL',
        'SWAP',
        'KW_NAMES',

        'POP_JUMP_FORWARD_IF_FALSE',
        'POP_JUMP_FORWARD_IF_TRUE',
        'POP_JUMP_FORWARD_IF_NOT_NONE',
        'POP_JUMP_FORWARD_IF_NONE',
        'POP_JUMP_BACKWARD_IF_NOT_NONE',
        'POP_JUMP_BACKWARD_IF_NONE',
        'POP_JUMP_BACKWARD_IF_FALSE',
        'POP_JUMP_BACKWARD_IF_TRUE',
        'FORMAT_VALUE',
        'BUILD_STRING',
    }

elif sys.version_info[0:2] == (3, 10):
    opcode_whitelist = {
        '<0>',
        '<8>',
        'POP_TOP',
        'ROT_TWO',
        'ROT_THREE',
        'DUP_TOP',
        'DUP_TOP_TWO',
        'ROT_FOUR',
        'NOP',
        'UNARY_POSITIVE',
        'UNARY_NEGATIVE',
        'UNARY_NOT',
        'UNARY_INVERT',
        'BINARY_MATRIX_MULTIPLY',
        'INPLACE_MATRIX_MULTIPLY',
        'BINARY_POWER',
        'BINARY_MULTIPLY',
        'BINARY_MODULO',
        'BINARY_ADD',
        'BINARY_SUBTRACT',
        'BINARY_SUBSCR',
        'BINARY_FLOOR_DIVIDE',
        'BINARY_TRUE_DIVIDE',
        'INPLACE_FLOOR_DIVIDE',
        'INPLACE_TRUE_DIVIDE',
        'INPLACE_ADD',
        'INPLACE_SUBTRACT',
        'INPLACE_MULTIPLY',
        'INPLACE_MODULO',
        'BINARY_LSHIFT',
        'BINARY_RSHIFT',
        'BINARY_AND',
        'BINARY_XOR',
        'BINARY_OR',
        'INPLACE_POWER',
        'INPLACE_LSHIFT',
        'INPLACE_RSHIFT',
        'INPLACE_AND',
        'INPLACE_XOR',
        'INPLACE_OR',
        'RETURN_VALUE',
        'BUILD_TUPLE',
        'BUILD_LIST',
        'BUILD_SET',
        'BUILD_MAP',
        'COMPARE_OP',
        'JUMP_FORWARD',
        'JUMP_IF_FALSE_OR_POP',
        'JUMP_IF_TRUE_OR_POP',
        'JUMP_ABSOLUTE',
        'POP_JUMP_IF_FALSE',
        'POP_JUMP_IF_TRUE',
        'LOAD_GLOBAL',
        'IS_OP',
        'LOAD_FAST',
        'STORE_FAST',
        'DELETE_FAST',
        'BUILD_SLICE',
        'LOAD_DEREF',
        'STORE_DEREF',
        'DELETE_DEREF',
        'LOAD_CONST',
        'LOAD_NAME',
        'CALL_FUNCTION',
        'CALL_FUNCTION_KW',
        'CALL_FUNCTION_EX',
        'FORMAT_VALUE',
        'BUILD_STRING',
    }

elif sys.version_info[0:2] == (3, 12):
    opcode_whitelist = {
        "RESUME",
        "CACHE",
        "LOAD_NAME",
        "LOAD_CONST",
        "LOAD_FAST",
        "LOAD_FAST_AND_CLEAR",
        "PUSH_NULL",
        "POP_TOP",
        "RETURN_VALUE",
        "RETURN_CONST",
        "CALL",
        "BINARY_OP",
        "BINARY_SUBSCR",
        "COMPARE_OP",
        "CONTAINS_OP",
        "POP_JUMP_IF_FALSE",
        "POP_JUMP_IF_TRUE",
        "JUMP_BACKWARD",
        "GET_ITER",
        "FOR_ITER",
        "END_FOR",
        "COPY",
        "SWAP",
        "FORMAT_VALUE",
        "BUILD_STRING",
        "BUILD_TUPLE",
        "BUILD_LIST",
        "BUILD_SET",
        "BUILD_MAP",
        "BUILD_CONST_KEY_MAP",
        "STORE_FAST",
        "LIST_APPEND",
        "LIST_EXTEND",
        "MAP_ADD",
        "SET_ADD",
        "SET_UPDATE",
        "UNARY_NEGATIVE",
        "UNARY_NOT",
        "RERAISE",
        }
    
else:
    raise RuntimeError("Python version not support!")
    

opname_reverse = {name: index for index, name in enumerate(dis.opname)}
try:
    opcode_whitelist_index = {opname_reverse[name] for name in opcode_whitelist}
except KeyError:
    opcode_whitelist_index = None

if opcode_whitelist_index is None:
    raise KeyError(
        "The following keys were not found: {!s}".format(
        list(sorted(name for name in opcode_whitelist if name not in opname_reverse)))
    )


def raise_if_code_unsafe(code, extra_globals=None, extra_locals=None):
    whitelist = set(builtins_whitelist)
    if extra_globals:
        whitelist.update(extra_globals)
    if extra_locals:
        whitelist.update(extra_locals)

    bad_ops = []
    for name in code.co_names:
        if name not in whitelist:
            bad_ops.append(name)

    if bad_ops:
        raise RuntimeError(
                "Name(s) %s not in white-list: (%s)" % (
                ", ".join(repr(name) for name in bad_ops),
                ", ".join(sorted(whitelist)))
                )

    for instr in dis.get_instructions(code):
        if instr.opcode not in opcode_whitelist_index:
            raise RuntimeError(
                "OpCode %r not in white-list %r" %
                (instr.opname, instr.opcode)
            )


def safe_eval(source, extra_globals=None):
    local_vars = {
        "sin": math.sin,
        "tan": math.tan,
        "cos": math.cos,
        "log": math.log,
        }

    code = compile(source, "<safe_eval>", "eval")

    raise_if_code_unsafe(code, extra_globals=extra_globals, extra_locals=local_vars)

    merged_globals = {}
    if extra_globals is not None:
        merged_globals.update(extra_globals)
    merged_globals.pop('__builtins__', None)

    ans = eval(code, merged_globals, local_vars)
    return ans
