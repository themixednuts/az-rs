mod support;

use az_lua_bytecode::decompile;

use support::{
    compile_file_bytes, compile_source_bytes, external_fixture, run_equivalence,
    run_stripped_equivalence,
};

const NUMERIC_FOR: &[u8] = include_bytes!("fixtures/control_flow/numeric_for.luac");
const WHILE_LOOP: &[u8] = include_bytes!("fixtures/control_flow/while.luac");
const REPEAT_LOOP: &[u8] = include_bytes!("fixtures/control_flow/repeat.luac");
const IF_ELSE_PHI: &[u8] = include_bytes!("fixtures/control_flow/if_else_phi.luac");
const IF_ELSEIF_ELSE: &[u8] = include_bytes!("fixtures/control_flow/if_elseif_else.luac");
const GENERIC_FOR: &[u8] = include_bytes!("fixtures/control_flow/generic_for.luac");
const NESTED_FOR_IF: &[u8] = include_bytes!("fixtures/control_flow/nested_for_if.luac");

#[test]
fn control_flow_fixtures_decompile_and_reparse() {
    let cases = [
        ("numeric_for", NUMERIC_FOR, &["for", "do", "end"][..]),
        ("while", WHILE_LOOP, &["while", "do", "end"][..]),
        ("repeat", REPEAT_LOOP, &["repeat", "until"][..]),
        ("if_else_phi", IF_ELSE_PHI, &["if", "else", "end"][..]),
        (
            "if_elseif_else",
            IF_ELSEIF_ELSE,
            &["if", "elseif", "else", "end"][..],
        ),
        ("generic_for", GENERIC_FOR, &["for", "in", "do", "end"][..]),
        ("nested_for_if", NESTED_FOR_IF, &["for", "if", "end"][..]),
    ];

    for (name, bytes, keywords) in cases {
        let source = decompile(bytes).unwrap_or_else(|error| panic!("{name}: {error}"));
        full_moon::parse(&source).unwrap_or_else(|errors| {
            panic!("{name}: emitted source did not parse:\n{source}\n{errors:#?}")
        });
        for keyword in keywords {
            assert!(
                source.contains(keyword),
                "{name}: expected keyword {keyword:?} in\n{source}"
            );
        }
    }
}

#[test]
fn if_else_phi_declares_once_and_assigns_in_both_arms() {
    let source = decompile(IF_ELSE_PHI).expect("decompile succeeds");
    assert_eq!(source.matches("local r").count(), 1, "{source}");
    assert!(source.contains("r = 1"), "{source}");
    assert!(source.contains("r = 2"), "{source}");
    assert!(source.contains("return r"), "{source}");
}

#[test]
fn runtime_equivalence_preserves_returning_if_elseif_fallthrough_chain() {
    let Some(decompiled) = run_equivalence(
        "returning_if_elseif_fallthrough_chain",
        r#"
local M = {}
function M:sel(a, b, x, y, z)
    if a then return x elseif b then return y end
    return z
end
print(M:sel(true, false, "x", "y", "z"))
print(M:sel(false, true, "x", "y", "z"))
print(M:sel(false, false, "x", "y", "z"))
"#,
    ) else {
        return;
    };

    assert!(decompiled.contains("if a then"), "{decompiled}");
    assert!(decompiled.contains("elseif b then"), "{decompiled}");
    assert!(decompiled.contains("return x"), "{decompiled}");
    assert!(decompiled.contains("return y"), "{decompiled}");
    assert!(decompiled.contains("return z"), "{decompiled}");
}

#[test]
fn condition_prefix_used_by_nested_branches_stays_in_the_else_scope() {
    let Some(decompiled) = run_stripped_equivalence(
        "condition_prefix_used_by_nested_branches",
        r#"
local function describe(kind, value)
    if kind == 1 then
        return "one"
    elseif kind == 2 then
        return "two"
    elseif string.match(value, "skip") then
        return "skip"
    else
        local fields = {
            audioBank = value,
            audioGroup = "group",
            audioState = "state",
        }
        if fields.audioBank ~= "" then
            return fields.audioBank
        elseif fields.audioGroup ~= "" and fields.audioState ~= "" then
            return fields.audioGroup .. fields.audioState
        end
    end
    return "empty"
end
print(describe(1, "bank"))
print(describe(2, "bank"))
print(describe(3, "skip-bank"))
print(describe(3, "bank"))
print(describe(3, ""))
"#,
    ) else {
        return;
    };

    assert!(
        decompiled.contains("local fields") || decompiled.contains("local l"),
        "the condition prefix must remain materialized:\n{decompiled}"
    );
    assert!(
        decompiled.contains("else\n") && decompiled.contains("audioBank"),
        "the nested condition must remain below its prefix:\n{decompiled}"
    );
}

#[test]
fn stripped_recursive_local_function_keeps_one_binding_identity() {
    let Some(decompiled) = run_stripped_equivalence(
        "recursive_local_function",
        r#"
local function collect(n, ...)
    if (n or 0) == 0 then
        return ...
    end
    return collect(n - 1, ...)
end
print(collect(3, "recursive", "binding"))
"#,
    ) else {
        return;
    };

    assert!(decompiled.contains("local function"), "{decompiled}");
    assert!(decompiled.contains("return l"), "{decompiled}");
}

#[test]
fn stripped_parameter_reassignments_keep_parameter_binding_identity() {
    let Some(decompiled) = run_stripped_equivalence(
        "parameter_reassignment",
        r"
local function fold(a, b)
    if b <= 0 then
        return a
    end
    local next_a = a + b
    local next_b = b - 1
    a = next_a
    b = next_b
    return fold(a, b)
end
print(fold(2, 3))
",
    ) else {
        return;
    };

    assert!(decompiled.contains("a0 ="), "{decompiled}");
    assert!(decompiled.contains("a1 ="), "{decompiled}");
}

#[test]
fn stripped_loop_exit_phi_keeps_initial_and_break_assignments_in_one_scope() {
    let Some(decompiled) = run_stripped_equivalence(
        "loop_exit_phi",
        r"
local function selected_offset(match_at)
    local offset = 0
    for index = 1, 5 do
        if index == match_at then
            offset = index * 10
            break
        end
    end
    return offset
end
print(selected_offset(3))
print(selected_offset(9))
",
    ) else {
        return;
    };

    assert!(decompiled.contains("local l"), "{decompiled}");
    assert!(decompiled.contains("break"), "{decompiled}");
}

#[test]
fn stripped_loop_search_results_remain_visible_after_nested_loops() {
    let Some(decompiled) = run_stripped_equivalence(
        "loop_search_results",
        r#"
local function find_pair(groups, left_key, right_key)
    local left
    local right
    for _, group in ipairs(groups) do
        for _, entry in ipairs(group) do
            if entry.key == left_key then
                left = entry
            elseif entry.key == right_key then
                right = entry
            end
        end
    end
    return left.value, right.value
end
local groups = {
    { { key = "left", value = 11 } },
    { { key = "right", value = 22 } },
}
print(find_pair(groups, "left", "right"))
"#,
    ) else {
        return;
    };

    assert!(decompiled.contains("return"), "{decompiled}");
}

#[test]
fn stripped_branch_assignment_inside_loop_remains_visible_after_loop() {
    let Some(decompiled) = run_stripped_equivalence(
        "branch_assignment_after_loop",
        r#"
local function find(values, wanted)
    local selected
    for _, value in ipairs(values) do
        if value.id == wanted then
            selected = value
        end
    end
    return selected.name
end
print(find({ { id = 1, name = "one" }, { id = 2, name = "two" } }, 2))
"#,
    ) else {
        return;
    };

    assert!(decompiled.contains("return"), "{decompiled}");
}

#[test]
fn stripped_per_iteration_capture_is_not_hoisted() {
    let Some(decompiled) = run_stripped_equivalence(
        "per_iteration_capture",
        r"
local callbacks = {}
for index = 1, 3 do
    local captured = index * 10
    callbacks[index] = function()
        return captured
    end
end
print(callbacks[1](), callbacks[2](), callbacks[3]())
",
    ) else {
        return;
    };

    assert!(decompiled.contains("function"), "{decompiled}");
}

#[test]
fn stripped_value_branch_before_unconditional_loop_break_keeps_both_arms() {
    let Some(decompiled) = run_stripped_equivalence(
        "value_branch_before_loop_break",
        r"
local function select_and_stop(values, use_left)
    local selected = 0
    for index = 1, #values do
        if values[index] > 0 then
            local base
            if use_left then
                base = values[index] * 2
            else
                base = values[index] * 3
            end
            selected = base + 1
            break
        end
    end
    return selected
end
print(select_and_stop({ 4, 5 }, true))
print(select_and_stop({ 4, 5 }, false))
",
    ) else {
        return;
    };

    assert!(decompiled.contains("else"), "{decompiled}");
    assert!(decompiled.contains("break"), "{decompiled}");
}

#[test]
fn stripped_value_region_preserves_sibling_conditional_assignments() {
    let Some(decompiled) = run_stripped_equivalence(
        "value_region_sibling_assignments",
        r"
local function flags(is_solo, mutable)
    local expedition = not mutable
    local mutation = mutable
    if not is_solo then
        mutable = false
        expedition = not mutable
        mutation = mutable
    end
    return expedition, mutation, mutable
end
print(flags(true, true))
print(flags(false, true))
",
    ) else {
        return;
    };

    assert!(decompiled.contains("if not"), "{decompiled}");
    assert!(decompiled.contains("return"), "{decompiled}");
}

#[test]
fn lua51_legacy_vararg_table_is_reified_with_exact_count() {
    let Some(decompiled) = run_equivalence(
        "legacy_vararg_table",
        r#"
local function summarize(...)
    local values = {}
    for index, value in ipairs(arg) do
        values[index] = value
    end
    return arg.n, table.concat(values, ":")
end
print(summarize("one", "two", "three"))
"#,
    ) else {
        return;
    };

    assert!(decompiled.contains("local arg"), "{decompiled}");
    assert!(decompiled.contains("select(\"#\", ...)"), "{decompiled}");
}

#[test]
fn guarded_value_with_non_false_fallback_stays_in_control_scope() {
    let Some(decompiled) = run_stripped_equivalence(
        "guarded_value_non_false_fallback",
        r"
local function perk_limit(display_all, tier_available)
    local count = 5
    if not display_all then
        local maximum = tier_available and 3 or 0
        if maximum < count and maximum then
            count = maximum
        end
    end
    return count
end
print(perk_limit(true, true))
print(perk_limit(false, true))
print(perk_limit(false, false))
",
    ) else {
        return;
    };

    assert!(decompiled.contains("if not"), "{decompiled}");
    assert!(decompiled.contains("return"), "{decompiled}");
}

#[test]
fn runtime_equivalence_preserves_loop_if_body_with_empty_continue_arm() {
    let Some(decompiled) = run_equivalence(
        "loop_if_body_empty_continue_arm",
        r#"
local function probe(values)
    local i = 1
    local out = ""
    while values[i] do
        local keepGoing = values[i] == "keep"
        i = i + 1
        if not keepGoing then
            out = out .. "x"
        end
    end
    print(out)
end
probe({ "keep", "stop", "keep" })
"#,
    ) else {
        return;
    };

    assert!(
        decompiled.contains("out = out .. \"x\""),
        "loop-local if body must not be dropped:\n{decompiled}"
    );
    assert!(
        !decompiled.contains("break"),
        "empty loop backedge must not become break:\n{decompiled}"
    );
}

#[test]
fn runtime_equivalence_preserves_guarded_early_return_body() {
    let Some(decompiled) = run_equivalence(
        "guarded_early_return_body",
        r#"
local M = {}
function M:guard(n, omit)
    if n <= 0 then return omit and "z" or "zzz" end
    return "pos"
end
print(M:guard(0, true))
print(M:guard(0, false))
print(M:guard(2, false))
"#,
    ) else {
        return;
    };

    assert!(
        decompiled.contains("if n <= 0 then") || decompiled.contains("if n > 0 then"),
        "{decompiled}"
    );
    assert!(
        decompiled.contains("return omit and \"z\" or \"zzz\""),
        "{decompiled}"
    );
    assert!(decompiled.contains("return \"pos\""), "{decompiled}");
    assert!(!decompiled.contains("if n <= 0 then\nend"), "{decompiled}");
}

#[test]
fn runtime_equivalence_preserves_nested_returning_branch_inside_else() {
    let Some(decompiled) = run_equivalence(
        "nested_returning_branch_inside_else",
        r#"
local M = {}
function M:nested(flag, a, b)
    if flag then return a
    else
        if a > b then return "hi" end
        return "lo"
    end
end
print(M:nested(true, "yes", "no"))
print(M:nested(false, 3, 1))
print(M:nested(false, 1, 3))
"#,
    ) else {
        return;
    };

    assert!(decompiled.contains("if flag then"), "{decompiled}");
    assert!(decompiled.contains("return a"), "{decompiled}");
    assert!(decompiled.contains("return \"hi\""), "{decompiled}");
    assert!(decompiled.contains("return \"lo\""), "{decompiled}");
}

#[test]
fn structural_returning_branch_patterns_decompile_with_all_returns() {
    let Some(bytecode) = compile_source_bytes(
        "structural_returning_branch_patterns",
        r#"
local M = {}
function M:sel(a, b, x, y, z)
    if a then return x elseif b then return y end
    return z
end
function M:guard(n, omit)
    if n <= 0 then return omit and "z" or "zzz" end
    return "pos"
end
function M:nested(flag, a, b)
    if flag then return a
    else
        if a > b then return "hi" end
        return "lo"
    end
end
return M
"#,
        false,
    ) else {
        return;
    };
    let decompiled = decompile(&bytecode).expect("minimal branch patterns decompile");

    assert!(decompiled.contains("elseif b then"), "{decompiled}");
    for expected in [
        "return x",
        "return y",
        "return z",
        "return omit and \"z\" or \"zzz\"",
        "return \"pos\"",
        "return a",
        "return \"hi\"",
        "return \"lo\"",
    ] {
        assert!(
            decompiled.contains(expected),
            "expected {expected:?} in\n{decompiled}"
        );
    }
    assert!(!decompiled.contains("then\nend"), "{decompiled}");
}

#[test]
fn background_path_preserves_returning_elseif_chain() {
    let Some(path) = external_fixture("lyshineui/_common/abilitiescommon.lua") else {
        return;
    };
    let Some(bytecode) = compile_file_bytes("abilitiescommon_background_path", &path, false) else {
        return;
    };
    let decompiled = decompile(&bytecode).expect("abilitiescommon decompiles");

    assert!(
        decompiled.contains("if useInfoOnlyPaths then"),
        "{decompiled}"
    );
    assert!(
        decompiled.contains("elseif usePassivePaths then"),
        "{decompiled}"
    );
    assert!(
        decompiled.contains("infoOnlyBackgroundPathByCategory")
            && decompiled.contains("passiveBackgroundPathByCategory")
            && decompiled.contains("backgroundPathByCategory"),
        "{decompiled}"
    );
    let function_body = decompiled
        .split("function AbilitiesCommon:GetBackgroundPath")
        .nth(1)
        .unwrap_or(&decompiled);
    assert!(
        function_body.matches("return ").count() >= 3,
        "expected all three branch returns to survive:\n{decompiled}"
    );
}

#[test]
fn time_helper_preserves_guarded_returns_and_nested_body() {
    let Some(path) = external_fixture("lyshineui/_common/timehelperfunctions.lua") else {
        return;
    };
    let Some(bytecode) = compile_file_bytes("timehelper_convert_seconds", &path, false) else {
        return;
    };
    let decompiled = decompile(&bytecode).expect("timehelperfunctions decompiles");

    assert!(decompiled.contains("if seconds <= 0 then"), "{decompiled}");
    assert!(
        decompiled.contains("return omitZeros and \"00\" or \"00:00:00\""),
        "{decompiled}"
    );
    assert!(decompiled.contains("if not omitZeros then"), "{decompiled}");
    assert!(decompiled.contains("local timeString"), "{decompiled}");
    assert!(
        decompiled.contains("return showDays and days .. \":\" .. timeString or timeString"),
        "{decompiled}"
    );
    assert!(decompiled.contains("return outString"), "{decompiled}");
    assert!(
        !decompiled.contains("if seconds <= 0 then\nend"),
        "{decompiled}"
    );
    assert!(
        !decompiled.contains("if not omitZeros then\nend"),
        "{decompiled}"
    );
}

#[test]
fn stripped_if_phi_keeps_shared_continuation_inside_numeric_loop() {
    let Some(decompiled) = run_stripped_equivalence(
        "if_phi_before_numeric_loop_control",
        r#"
local function render(rows)
    for index = 1, #rows do
        local row = rows[index]
        if row.visible then
            local label = nil
            if row.value then
                label = row.value .. "!"
            end
            print(index, label)
        end
    end
end
render({
    { visible = true, value = "one" },
    { visible = true },
    { visible = false, value = "hidden" },
})
"#,
    ) else {
        return;
    };

    assert!(
        decompiled.contains("print("),
        "the PHI consumer must remain in the shared continuation:\n{decompiled}"
    );
    assert!(
        decompiled.contains("for ") && decompiled.contains(" do"),
        "the numeric loop must remain structurally intact:\n{decompiled}"
    );
}

#[test]
fn stripped_lua51_legacy_vararg_table_follows_fixed_parameters() {
    let Some(decompiled) = run_stripped_equivalence(
        "legacy_vararg_table_after_fixed_parameter",
        r#"
local function summarize(prefix, ...)
    print(prefix, arg.n, arg[1], arg[2])
end
summarize("values", "one", "two")
"#,
    ) else {
        return;
    };

    assert!(decompiled.contains("local arg"), "{decompiled}");
    assert!(decompiled.contains("select(\"#\", ...)"), "{decompiled}");
}

#[test]
fn stripped_multi_return_fallback_keeps_one_shared_binding() {
    let Some(decompiled) = run_stripped_equivalence(
        "multi_return_fallback",
        r#"
local function pick(primary, fallback)
    local _, value = next(primary)
    if not value then
        _, value = next(fallback)
    end
    return value
end
print(pick({ first = "primary" }, { second = "fallback" }))
print(pick({}, { second = "fallback" }))
"#,
    ) else {
        return;
    };

    assert!(decompiled.contains("next("), "{decompiled}");
    assert!(decompiled.contains("return "), "{decompiled}");
}
