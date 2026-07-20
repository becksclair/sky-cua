import {
  parse,
  type Pattern,
  type Program,
  type Statement,
  type VariableDeclaration,
} from "acorn";

export type PersistentBindingKind =
  | "var"
  | "let"
  | "const"
  | "function"
  | "class";

export interface PersistentBinding {
  kind: PersistentBindingKind;
  name: string;
}

const STATIC_IMPORT_ERROR =
  'Top-level static import is not supported in node_repl. Use await import("...") instead.';
const TOP_LEVEL_EXPORT_ERROR =
  "Top-level export is not supported in node_repl cells";

export function analyzeCell(source: string): PersistentBinding[] {
  const program = parse(source, {
    ecmaVersion: "latest",
    sourceType: "module",
  });
  rejectStaticModuleDeclarations(program);

  const bindings: PersistentBinding[] = [];
  for (const statement of program.body) {
    if (
      statement.type === "ImportDeclaration" ||
      statement.type === "ExportNamedDeclaration" ||
      statement.type === "ExportDefaultDeclaration" ||
      statement.type === "ExportAllDeclaration"
    ) {
      continue;
    }
    if (statement.type === "VariableDeclaration") {
      collectVariableDeclaration(statement, bindings);
    } else if (statement.type === "FunctionDeclaration") {
      bindings.push({ kind: "function", name: statement.id.name });
    } else if (statement.type === "ClassDeclaration") {
      bindings.push({ kind: "class", name: statement.id.name });
    } else {
      collectNestedVarDeclarations(statement, bindings);
    }
  }
  return bindings;
}

function collectNestedVarDeclarations(
  statement: Statement,
  bindings: PersistentBinding[],
): void {
  if (statement.type === "VariableDeclaration") {
    if (statement.kind === "var")
      collectVariableDeclaration(statement, bindings);
    return;
  }
  if (statement.type === "BlockStatement") {
    for (const child of statement.body)
      collectNestedVarDeclarations(child, bindings);
    return;
  }
  if (statement.type === "IfStatement") {
    collectNestedVarDeclarations(statement.consequent, bindings);
    if (statement.alternate !== null && statement.alternate !== undefined)
      collectNestedVarDeclarations(statement.alternate, bindings);
    return;
  }
  if (
    statement.type === "LabeledStatement" ||
    statement.type === "WhileStatement" ||
    statement.type === "DoWhileStatement"
  ) {
    collectNestedVarDeclarations(statement.body, bindings);
    return;
  }
  if (statement.type === "ForStatement") {
    if (
      statement.init?.type === "VariableDeclaration" &&
      statement.init.kind === "var"
    )
      collectVariableDeclaration(statement.init, bindings);
    collectNestedVarDeclarations(statement.body, bindings);
    return;
  }
  if (
    statement.type === "ForInStatement" ||
    statement.type === "ForOfStatement"
  ) {
    if (
      statement.left.type === "VariableDeclaration" &&
      statement.left.kind === "var"
    )
      collectVariableDeclaration(statement.left, bindings);
    collectNestedVarDeclarations(statement.body, bindings);
    return;
  }
  if (statement.type === "SwitchStatement") {
    for (const switchCase of statement.cases) {
      for (const child of switchCase.consequent)
        collectNestedVarDeclarations(child, bindings);
    }
    return;
  }
  if (statement.type === "TryStatement") {
    collectNestedVarDeclarations(statement.block, bindings);
    if (statement.handler !== null && statement.handler !== undefined)
      collectNestedVarDeclarations(statement.handler.body, bindings);
    if (statement.finalizer !== null && statement.finalizer !== undefined)
      collectNestedVarDeclarations(statement.finalizer, bindings);
  }
}

function rejectStaticModuleDeclarations(program: Program): void {
  for (const statement of program.body) {
    if (statement.type === "ImportDeclaration")
      throw new Error(STATIC_IMPORT_ERROR);
    if (
      statement.type === "ExportNamedDeclaration" ||
      statement.type === "ExportDefaultDeclaration" ||
      statement.type === "ExportAllDeclaration"
    ) {
      throw new Error(TOP_LEVEL_EXPORT_ERROR);
    }
  }
}

function collectVariableDeclaration(
  declaration: VariableDeclaration,
  bindings: PersistentBinding[],
): void {
  if (
    declaration.kind !== "var" &&
    declaration.kind !== "let" &&
    declaration.kind !== "const"
  ) {
    return;
  }
  for (const declarator of declaration.declarations) {
    collectPatternBindings(declarator.id, declaration.kind, bindings);
  }
}

function collectPatternBindings(
  pattern: Pattern,
  kind: "var" | "let" | "const",
  bindings: PersistentBinding[],
): void {
  if (pattern.type === "Identifier") {
    bindings.push({ kind, name: pattern.name });
    return;
  }
  if (pattern.type === "ObjectPattern") {
    for (const property of pattern.properties) {
      if (property.type === "RestElement") {
        collectPatternBindings(property.argument, kind, bindings);
      } else {
        collectPatternBindings(property.value, kind, bindings);
      }
    }
    return;
  }
  if (pattern.type === "ArrayPattern") {
    for (const element of pattern.elements) {
      if (element !== null) collectPatternBindings(element, kind, bindings);
    }
    return;
  }
  if (pattern.type === "RestElement") {
    collectPatternBindings(pattern.argument, kind, bindings);
    return;
  }
  if (pattern.type === "AssignmentPattern") {
    collectPatternBindings(pattern.left, kind, bindings);
    return;
  }
  throw new SyntaxError("Invalid variable binding pattern");
}
