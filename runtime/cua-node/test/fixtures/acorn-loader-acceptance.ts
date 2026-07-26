export const ACORN_LOADER_ACCEPTANCE_CODE =
  'var acornAcceptance = await nodeRepl.loaders.acorn(); var acornAcceptanceImport = await import("acorn"); var walkAcceptance = await nodeRepl.loaders.acornWalk(); var walkAcceptanceImport = await import("acorn-walk"); var programAcceptance = acornAcceptance.parse("const answer = () => 42", {ecmaVersion:"latest",sourceType:"module"}); var identifiersAcceptance = []; walkAcceptance.simple(programAcceptance, {VariablePattern:function(node){identifiersAcceptance.push(node.name)}}); nodeRepl.write(JSON.stringify({acorn_identity:acornAcceptance===acornAcceptanceImport,walk_identity:walkAcceptance===walkAcceptanceImport,source_type:programAcceptance.sourceType,identifiers:identifiersAcceptance.join(",")}))';

export const ACORN_LOADER_ACCEPTANCE_RESULT = Object.freeze({
  acorn_identity: true,
  walk_identity: true,
  source_type: "module",
  identifiers: "answer",
});
