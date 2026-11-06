(function(){const e=document.createElement("link").relList;if(e&&e.supports&&e.supports("modulepreload"))return;for(const a of document.querySelectorAll('link[rel="modulepreload"]'))s(a);new MutationObserver(a=>{for(const r of a)if(r.type==="childList")for(const i of r.addedNodes)i.tagName==="LINK"&&i.rel==="modulepreload"&&s(i)}).observe(document,{childList:!0,subtree:!0});function t(a){const r={};return a.integrity&&(r.integrity=a.integrity),a.referrerPolicy&&(r.referrerPolicy=a.referrerPolicy),a.crossOrigin==="use-credentials"?r.credentials="include":a.crossOrigin==="anonymous"?r.credentials="omit":r.credentials="same-origin",r}function s(a){if(a.ep)return;a.ep=!0;const r=t(a);fetch(a.href,r)}})();function d(n){return typeof n!="object"||n==null||!("id"in n)||typeof n.id!="string"||!("type"in n)?!1:n.type==="js-kernel:cancel"||n.type==="js-kernel:reset"?!0:n.type==="js-kernel:execute"&&"script"in n&&typeof n.script=="string"}function l(n){return typeof n=="object"&&n!=null&&"id"in n&&typeof n.id=="string"&&"ok"in n&&typeof n.ok=="boolean"}function u(n){return n instanceof Error?{message:n.message,name:n.name,stack:n.stack}:{message:typeof n=="string"?n:"JavaScript execution failed",name:"Error"}}var c=`(function() {
	var Dt = Object.defineProperty, l = (e, t) => {
		let a = {};
		for (var r in e) Dt(a, r, {
			get: e[r],
			enumerable: !0
		});
		return t || Dt(a, Symbol.toStringTag, { value: "Module" }), a;
	};
	const Kc = (e) => {
		if (e === null) return null;
		if (typeof e == "string" || typeof e == "boolean") return e;
		if (typeof e == "number") return Number.isFinite(e) ? e : void 0;
		if (e instanceof String || e instanceof Boolean) return e.valueOf();
		if (e instanceof Number) {
			const t = e.valueOf();
			return Number.isFinite(t) ? t : void 0;
		}
	}, Yc = (e) => typeof e == "number" && Number.isInteger(e) && e > 0, Ot = (e, t) => {
		if (!Yc(t)) return e;
		const a = t;
		if (e.length <= a) return e;
		const r = e.length - a;
		return \`\${e.slice(0, a)}[truncated \${r} chars]\`;
	}, Xc = (e, t) => {
		switch (e.type) {
			case "function": return e;
			case "error":
			case "object": return {
				...e,
				value: Ot(e.value, t)
			};
			case "value": return typeof e.value != "string" ? e : {
				...e,
				value: Ot(e.value, t)
			};
		}
	}, Qc = async (e, t, a) => {
		const { displayImage: r, displayValue: s } = e;
		let c = null;
		try {
			if (t instanceof Uint8Array) {
				await r(t);
				return;
			} else if (t === void 0) c = {
				type: "value",
				value: void 0
			};
			else if (typeof t == "function") c = {
				type: "function",
				value: void 0
			};
			else {
				const o = Kc(t);
				o !== void 0 && (c = {
					type: "value",
					value: o
				});
			}
			if (c === null) {
				let o = JSON.stringify(t);
				o === void 0 && (o = String(t)), c = {
					type: "object",
					value: o
				};
			}
		} catch (o) {
			c = {
				type: "error",
				value: String(o)
			};
		}
		c && await s(Xc(c, a));
	};
	function ed({ displayBridge: e, displayTruncateMaxChars: t }) {
		return (a) => Qc(e, a, t);
	}
	var h = class {
		constructor(e, t, a) {
			this.type = e, this.schema = t, this.payload = a;
		}
		parse() {
			return this.schema.parse(this.payload);
		}
		toJSON() {
			return {
				type: this.type,
				...this.payload
			};
		}
	}, $;
	(function(e) {
		e.assertEqual = (s) => {};
		function t(s) {}
		e.assertIs = t;
		function a(s) {
			throw new Error();
		}
		e.assertNever = a, e.arrayToEnum = (s) => {
			const c = {};
			for (const o of s) c[o] = o;
			return c;
		}, e.getValidEnumValues = (s) => {
			const c = e.objectKeys(s).filter((d) => typeof s[s[d]] != "number"), o = {};
			for (const d of c) o[d] = s[d];
			return e.objectValues(o);
		}, e.objectValues = (s) => e.objectKeys(s).map(function(c) {
			return s[c];
		}), e.objectKeys = typeof Object.keys == "function" ? (s) => Object.keys(s) : (s) => {
			const c = [];
			for (const o in s) Object.prototype.hasOwnProperty.call(s, o) && c.push(o);
			return c;
		}, e.find = (s, c) => {
			for (const o of s) if (c(o)) return o;
		}, e.isInteger = typeof Number.isInteger == "function" ? (s) => Number.isInteger(s) : (s) => typeof s == "number" && Number.isFinite(s) && Math.floor(s) === s;
		function r(s, c = " | ") {
			return s.map((o) => typeof o == "string" ? \`'\${o}'\` : o).join(c);
		}
		e.joinValues = r, e.jsonStringifyReplacer = (s, c) => typeof c == "bigint" ? c.toString() : c;
	})($ || ($ = {}));
	var Mt;
	(function(e) {
		e.mergeShapes = (t, a) => ({
			...t,
			...a
		});
	})(Mt || (Mt = {}));
	const _ = $.arrayToEnum([
		"string",
		"nan",
		"number",
		"integer",
		"float",
		"boolean",
		"date",
		"bigint",
		"symbol",
		"function",
		"undefined",
		"null",
		"array",
		"object",
		"unknown",
		"promise",
		"void",
		"never",
		"map",
		"set"
	]), W = (e) => {
		switch (typeof e) {
			case "undefined": return _.undefined;
			case "string": return _.string;
			case "number": return Number.isNaN(e) ? _.nan : _.number;
			case "boolean": return _.boolean;
			case "function": return _.function;
			case "bigint": return _.bigint;
			case "symbol": return _.symbol;
			case "object": return Array.isArray(e) ? _.array : e === null ? _.null : e.then && typeof e.then == "function" && e.catch && typeof e.catch == "function" ? _.promise : typeof Map < "u" && e instanceof Map ? _.map : typeof Set < "u" && e instanceof Set ? _.set : typeof Date < "u" && e instanceof Date ? _.date : _.object;
			default: return _.unknown;
		}
	}, m = $.arrayToEnum([
		"invalid_type",
		"invalid_literal",
		"custom",
		"invalid_union",
		"invalid_union_discriminator",
		"invalid_enum_value",
		"unrecognized_keys",
		"invalid_arguments",
		"invalid_return_type",
		"invalid_date",
		"invalid_string",
		"too_small",
		"too_big",
		"invalid_intersection_types",
		"not_multiple_of",
		"not_finite"
	]);
	var j = class Fc extends Error {
		get errors() {
			return this.issues;
		}
		constructor(t) {
			super(), this.issues = [], this.addIssue = (r) => {
				this.issues = [...this.issues, r];
			}, this.addIssues = (r = []) => {
				this.issues = [...this.issues, ...r];
			};
			const a = new.target.prototype;
			Object.setPrototypeOf ? Object.setPrototypeOf(this, a) : this.__proto__ = a, this.name = "ZodError", this.issues = t;
		}
		format(t) {
			const a = t || function(c) {
				return c.message;
			}, r = { _errors: [] }, s = (c) => {
				for (const o of c.issues) if (o.code === "invalid_union") o.unionErrors.map(s);
				else if (o.code === "invalid_return_type") s(o.returnTypeError);
				else if (o.code === "invalid_arguments") s(o.argumentsError);
				else if (o.path.length === 0) r._errors.push(a(o));
				else {
					let d = r, p = 0;
					for (; p < o.path.length;) {
						const f = o.path[p];
						p !== o.path.length - 1 ? d[f] = d[f] || { _errors: [] } : (d[f] = d[f] || { _errors: [] }, d[f]._errors.push(a(o))), d = d[f], p++;
					}
				}
			};
			return s(this), r;
		}
		static assert(t) {
			if (!(t instanceof Fc)) throw new Error(\`Not a ZodError: \${t}\`);
		}
		toString() {
			return this.message;
		}
		get message() {
			return JSON.stringify(this.issues, $.jsonStringifyReplacer, 2);
		}
		get isEmpty() {
			return this.issues.length === 0;
		}
		flatten(t = (a) => a.message) {
			const a = {}, r = [];
			for (const s of this.issues) if (s.path.length > 0) {
				const c = s.path[0];
				a[c] = a[c] || [], a[c].push(t(s));
			} else r.push(t(s));
			return {
				formErrors: r,
				fieldErrors: a
			};
		}
		get formErrors() {
			return this.flatten();
		}
	};
	j.create = (e) => new j(e);
	const ie = (e, t) => {
		let a;
		switch (e.code) {
			case m.invalid_type:
				e.received === _.undefined ? a = "Required" : a = \`Expected \${e.expected}, received \${e.received}\`;
				break;
			case m.invalid_literal:
				a = \`Invalid literal value, expected \${JSON.stringify(e.expected, $.jsonStringifyReplacer)}\`;
				break;
			case m.unrecognized_keys:
				a = \`Unrecognized key(s) in object: \${$.joinValues(e.keys, ", ")}\`;
				break;
			case m.invalid_union:
				a = "Invalid input";
				break;
			case m.invalid_union_discriminator:
				a = \`Invalid discriminator value. Expected \${$.joinValues(e.options)}\`;
				break;
			case m.invalid_enum_value:
				a = \`Invalid enum value. Expected \${$.joinValues(e.options)}, received '\${e.received}'\`;
				break;
			case m.invalid_arguments:
				a = "Invalid function arguments";
				break;
			case m.invalid_return_type:
				a = "Invalid function return type";
				break;
			case m.invalid_date:
				a = "Invalid date";
				break;
			case m.invalid_string:
				typeof e.validation == "object" ? "includes" in e.validation ? (a = \`Invalid input: must include "\${e.validation.includes}"\`, typeof e.validation.position == "number" && (a = \`\${a} at one or more positions greater than or equal to \${e.validation.position}\`)) : "startsWith" in e.validation ? a = \`Invalid input: must start with "\${e.validation.startsWith}"\` : "endsWith" in e.validation ? a = \`Invalid input: must end with "\${e.validation.endsWith}"\` : $.assertNever(e.validation) : e.validation !== "regex" ? a = \`Invalid \${e.validation}\` : a = "Invalid";
				break;
			case m.too_small:
				e.type === "array" ? a = \`Array must contain \${e.exact ? "exactly" : e.inclusive ? "at least" : "more than"} \${e.minimum} element(s)\` : e.type === "string" ? a = \`String must contain \${e.exact ? "exactly" : e.inclusive ? "at least" : "over"} \${e.minimum} character(s)\` : e.type === "number" ? a = \`Number must be \${e.exact ? "exactly equal to " : e.inclusive ? "greater than or equal to " : "greater than "}\${e.minimum}\` : e.type === "bigint" ? a = \`Number must be \${e.exact ? "exactly equal to " : e.inclusive ? "greater than or equal to " : "greater than "}\${e.minimum}\` : e.type === "date" ? a = \`Date must be \${e.exact ? "exactly equal to " : e.inclusive ? "greater than or equal to " : "greater than "}\${new Date(Number(e.minimum))}\` : a = "Invalid input";
				break;
			case m.too_big:
				e.type === "array" ? a = \`Array must contain \${e.exact ? "exactly" : e.inclusive ? "at most" : "less than"} \${e.maximum} element(s)\` : e.type === "string" ? a = \`String must contain \${e.exact ? "exactly" : e.inclusive ? "at most" : "under"} \${e.maximum} character(s)\` : e.type === "number" ? a = \`Number must be \${e.exact ? "exactly" : e.inclusive ? "less than or equal to" : "less than"} \${e.maximum}\` : e.type === "bigint" ? a = \`BigInt must be \${e.exact ? "exactly" : e.inclusive ? "less than or equal to" : "less than"} \${e.maximum}\` : e.type === "date" ? a = \`Date must be \${e.exact ? "exactly" : e.inclusive ? "smaller than or equal to" : "smaller than"} \${new Date(Number(e.maximum))}\` : a = "Invalid input";
				break;
			case m.custom:
				a = "Invalid input";
				break;
			case m.invalid_intersection_types:
				a = "Intersection results could not be merged";
				break;
			case m.not_multiple_of:
				a = \`Number must be a multiple of \${e.multipleOf}\`;
				break;
			case m.not_finite:
				a = "Number must be finite";
				break;
			default: a = t.defaultError, $.assertNever(e);
		}
		return { message: a };
	};
	let td = ie;
	function qe() {
		return td;
	}
	const Ue = (e) => {
		const { data: t, path: a, errorMaps: r, issueData: s } = e, c = [...a, ...s.path || []], o = {
			...s,
			path: c
		};
		if (s.message !== void 0) return {
			...s,
			path: c,
			message: s.message
		};
		let d = "";
		const p = r.filter((f) => !!f).slice().reverse();
		for (const f of p) d = f(o, {
			data: t,
			defaultError: d
		}).message;
		return {
			...s,
			path: c,
			message: d
		};
	};
	function y(e, t) {
		const a = qe(), r = Ue({
			issueData: t,
			data: e.data,
			path: e.path,
			errorMaps: [
				e.common.contextualErrorMap,
				e.schemaErrorMap,
				a,
				a === ie ? void 0 : ie
			].filter((s) => !!s)
		});
		e.common.issues.push(r);
	}
	var N = class Vc {
		constructor() {
			this.value = "valid";
		}
		dirty() {
			this.value === "valid" && (this.value = "dirty");
		}
		abort() {
			this.value !== "aborted" && (this.value = "aborted");
		}
		static mergeArray(t, a) {
			const r = [];
			for (const s of a) {
				if (s.status === "aborted") return w;
				s.status === "dirty" && t.dirty(), r.push(s.value);
			}
			return {
				status: t.value,
				value: r
			};
		}
		static async mergeObjectAsync(t, a) {
			const r = [];
			for (const s of a) {
				const c = await s.key, o = await s.value;
				r.push({
					key: c,
					value: o
				});
			}
			return Vc.mergeObjectSync(t, r);
		}
		static mergeObjectSync(t, a) {
			const r = {};
			for (const s of a) {
				const { key: c, value: o } = s;
				if (c.status === "aborted" || o.status === "aborted") return w;
				c.status === "dirty" && t.dirty(), o.status === "dirty" && t.dirty(), c.value !== "__proto__" && (typeof o.value < "u" || s.alwaysSet) && (r[c.value] = o.value);
			}
			return {
				status: t.value,
				value: r
			};
		}
	};
	const w = Object.freeze({ status: "aborted" }), ne = (e) => ({
		status: "dirty",
		value: e
	}), E = (e) => ({
		status: "valid",
		value: e
	}), Bt = (e) => e.status === "aborted", jt = (e) => e.status === "dirty", Y = (e) => e.status === "valid", pe = (e) => typeof Promise < "u" && e instanceof Promise;
	var b;
	(function(e) {
		e.errToObj = (t) => typeof t == "string" ? { message: t } : t || {}, e.toString = (t) => typeof t == "string" ? t : t?.message;
	})(b || (b = {}));
	var L = class {
		constructor(e, t, a, r) {
			this._cachedPath = [], this.parent = e, this.data = t, this._path = a, this._key = r;
		}
		get path() {
			return this._cachedPath.length || (Array.isArray(this._key) ? this._cachedPath.push(...this._path, ...this._key) : this._cachedPath.push(...this._path, this._key)), this._cachedPath;
		}
	};
	const Lt = (e, t) => {
		if (Y(t)) return {
			success: !0,
			data: t.value
		};
		if (!e.common.issues.length) throw new Error("Validation failed but no issues detected.");
		return {
			success: !1,
			get error() {
				if (this._error) return this._error;
				const a = new j(e.common.issues);
				return this._error = a, this._error;
			}
		};
	};
	function x(e) {
		if (!e) return {};
		const { errorMap: t, invalid_type_error: a, required_error: r, description: s } = e;
		if (t && (a || r)) throw new Error(\`Can't use "invalid_type_error" or "required_error" in conjunction with custom error map.\`);
		return t ? {
			errorMap: t,
			description: s
		} : {
			errorMap: (o, d) => {
				const { message: p } = e;
				return o.code === "invalid_enum_value" ? { message: p ?? d.defaultError } : typeof d.data > "u" ? { message: p ?? r ?? d.defaultError } : o.code !== "invalid_type" ? { message: d.defaultError } : { message: p ?? a ?? d.defaultError };
			},
			description: s
		};
	}
	var T = class {
		get description() {
			return this._def.description;
		}
		_getType(e) {
			return W(e.data);
		}
		_getOrReturnCtx(e, t) {
			return t || {
				common: e.parent.common,
				data: e.data,
				parsedType: W(e.data),
				schemaErrorMap: this._def.errorMap,
				path: e.path,
				parent: e.parent
			};
		}
		_processInputParams(e) {
			return {
				status: new N(),
				ctx: {
					common: e.parent.common,
					data: e.data,
					parsedType: W(e.data),
					schemaErrorMap: this._def.errorMap,
					path: e.path,
					parent: e.parent
				}
			};
		}
		_parseSync(e) {
			const t = this._parse(e);
			if (pe(t)) throw new Error("Synchronous parse encountered promise.");
			return t;
		}
		_parseAsync(e) {
			const t = this._parse(e);
			return Promise.resolve(t);
		}
		parse(e, t) {
			const a = this.safeParse(e, t);
			if (a.success) return a.data;
			throw a.error;
		}
		safeParse(e, t) {
			const a = {
				common: {
					issues: [],
					async: t?.async ?? !1,
					contextualErrorMap: t?.errorMap
				},
				path: t?.path || [],
				schemaErrorMap: this._def.errorMap,
				parent: null,
				data: e,
				parsedType: W(e)
			};
			return Lt(a, this._parseSync({
				data: e,
				path: a.path,
				parent: a
			}));
		}
		"~validate"(e) {
			const t = {
				common: {
					issues: [],
					async: !!this["~standard"].async
				},
				path: [],
				schemaErrorMap: this._def.errorMap,
				parent: null,
				data: e,
				parsedType: W(e)
			};
			if (!this["~standard"].async) try {
				const a = this._parseSync({
					data: e,
					path: [],
					parent: t
				});
				return Y(a) ? { value: a.value } : { issues: t.common.issues };
			} catch (a) {
				a?.message?.toLowerCase()?.includes("encountered") && (this["~standard"].async = !0), t.common = {
					issues: [],
					async: !0
				};
			}
			return this._parseAsync({
				data: e,
				path: [],
				parent: t
			}).then((a) => Y(a) ? { value: a.value } : { issues: t.common.issues });
		}
		async parseAsync(e, t) {
			const a = await this.safeParseAsync(e, t);
			if (a.success) return a.data;
			throw a.error;
		}
		async safeParseAsync(e, t) {
			const a = {
				common: {
					issues: [],
					contextualErrorMap: t?.errorMap,
					async: !0
				},
				path: t?.path || [],
				schemaErrorMap: this._def.errorMap,
				parent: null,
				data: e,
				parsedType: W(e)
			}, r = this._parse({
				data: e,
				path: a.path,
				parent: a
			});
			return Lt(a, await (pe(r) ? r : Promise.resolve(r)));
		}
		refine(e, t) {
			const a = (r) => typeof t == "string" || typeof t > "u" ? { message: t } : typeof t == "function" ? t(r) : t;
			return this._refinement((r, s) => {
				const c = e(r), o = () => s.addIssue({
					code: m.custom,
					...a(r)
				});
				return typeof Promise < "u" && c instanceof Promise ? c.then((d) => d ? !0 : (o(), !1)) : c ? !0 : (o(), !1);
			});
		}
		refinement(e, t) {
			return this._refinement((a, r) => e(a) ? !0 : (r.addIssue(typeof t == "function" ? t(a, r) : t), !1));
		}
		_refinement(e) {
			return new V({
				schema: this,
				typeName: v.ZodEffects,
				effect: {
					type: "refinement",
					refinement: e
				}
			});
		}
		superRefine(e) {
			return this._refinement(e);
		}
		constructor(e) {
			this.spa = this.safeParseAsync, this._def = e, this.parse = this.parse.bind(this), this.safeParse = this.safeParse.bind(this), this.parseAsync = this.parseAsync.bind(this), this.safeParseAsync = this.safeParseAsync.bind(this), this.spa = this.spa.bind(this), this.refine = this.refine.bind(this), this.refinement = this.refinement.bind(this), this.superRefine = this.superRefine.bind(this), this.optional = this.optional.bind(this), this.nullable = this.nullable.bind(this), this.nullish = this.nullish.bind(this), this.array = this.array.bind(this), this.promise = this.promise.bind(this), this.or = this.or.bind(this), this.and = this.and.bind(this), this.transform = this.transform.bind(this), this.brand = this.brand.bind(this), this.default = this.default.bind(this), this.catch = this.catch.bind(this), this.describe = this.describe.bind(this), this.pipe = this.pipe.bind(this), this.readonly = this.readonly.bind(this), this.isNullable = this.isNullable.bind(this), this.isOptional = this.isOptional.bind(this), this["~standard"] = {
				version: 1,
				vendor: "zod",
				validate: (t) => this["~validate"](t)
			};
		}
		optional() {
			return H.create(this, this._def);
		}
		nullable() {
			return J.create(this, this._def);
		}
		nullish() {
			return this.nullable().optional();
		}
		array() {
			return Q.create(this);
		}
		promise() {
			return oe.create(this, this._def);
		}
		or(e) {
			return be.create([this, e], this._def);
		}
		and(e) {
			return ge.create(this, e, this._def);
		}
		transform(e) {
			return new V({
				...x(this._def),
				schema: this,
				typeName: v.ZodEffects,
				effect: {
					type: "transform",
					transform: e
				}
			});
		}
		default(e) {
			const t = typeof e == "function" ? e : () => e;
			return new Te({
				...x(this._def),
				innerType: this,
				defaultValue: t,
				typeName: v.ZodDefault
			});
		}
		brand() {
			return new Ft({
				typeName: v.ZodBranded,
				type: this,
				...x(this._def)
			});
		}
		catch(e) {
			const t = typeof e == "function" ? e : () => e;
			return new $e({
				...x(this._def),
				innerType: this,
				catchValue: t,
				typeName: v.ZodCatch
			});
		}
		describe(e) {
			const t = this.constructor;
			return new t({
				...this._def,
				description: e
			});
		}
		pipe(e) {
			return Vt.create(this, e);
		}
		readonly() {
			return ke.create(this);
		}
		isOptional() {
			return this.safeParse(void 0).success;
		}
		isNullable() {
			return this.safeParse(null).success;
		}
	};
	const ad = /^c[^\\s-]{8,}$/i, rd = /^[0-9a-z]+$/, sd = /^[0-9A-HJKMNP-TV-Z]{26}$/i, id = /^[0-9a-fA-F]{8}\\b-[0-9a-fA-F]{4}\\b-[0-9a-fA-F]{4}\\b-[0-9a-fA-F]{4}\\b-[0-9a-fA-F]{12}$/i, nd = /^[a-z0-9_-]{21}$/i, od = /^[A-Za-z0-9-_]+\\.[A-Za-z0-9-_]+\\.[A-Za-z0-9-_]*$/, cd = /^[-+]?P(?!$)(?:(?:[-+]?\\d+Y)|(?:[-+]?\\d+[.,]\\d+Y$))?(?:(?:[-+]?\\d+M)|(?:[-+]?\\d+[.,]\\d+M$))?(?:(?:[-+]?\\d+W)|(?:[-+]?\\d+[.,]\\d+W$))?(?:(?:[-+]?\\d+D)|(?:[-+]?\\d+[.,]\\d+D$))?(?:T(?=[\\d+-])(?:(?:[-+]?\\d+H)|(?:[-+]?\\d+[.,]\\d+H$))?(?:(?:[-+]?\\d+M)|(?:[-+]?\\d+[.,]\\d+M$))?(?:[-+]?\\d+(?:[.,]\\d+)?S)?)??$/, dd = /^(?!\\.)(?!.*\\.\\.)([A-Z0-9_'+\\-\\.]*)[A-Z0-9_+-]@([A-Z0-9][A-Z0-9\\-]*\\.)+[A-Z]{2,}$/i, ld = "^(\\\\p{Extended_Pictographic}|\\\\p{Emoji_Component})+$";
	let Fe;
	const ud = /^(?:(?:25[0-5]|2[0-4][0-9]|1[0-9][0-9]|[1-9][0-9]|[0-9])\\.){3}(?:25[0-5]|2[0-4][0-9]|1[0-9][0-9]|[1-9][0-9]|[0-9])$/, hd = /^(?:(?:25[0-5]|2[0-4][0-9]|1[0-9][0-9]|[1-9][0-9]|[0-9])\\.){3}(?:25[0-5]|2[0-4][0-9]|1[0-9][0-9]|[1-9][0-9]|[0-9])\\/(3[0-2]|[12]?[0-9])$/, md = /^(([0-9a-fA-F]{1,4}:){7,7}[0-9a-fA-F]{1,4}|([0-9a-fA-F]{1,4}:){1,7}:|([0-9a-fA-F]{1,4}:){1,6}:[0-9a-fA-F]{1,4}|([0-9a-fA-F]{1,4}:){1,5}(:[0-9a-fA-F]{1,4}){1,2}|([0-9a-fA-F]{1,4}:){1,4}(:[0-9a-fA-F]{1,4}){1,3}|([0-9a-fA-F]{1,4}:){1,3}(:[0-9a-fA-F]{1,4}){1,4}|([0-9a-fA-F]{1,4}:){1,2}(:[0-9a-fA-F]{1,4}){1,5}|[0-9a-fA-F]{1,4}:((:[0-9a-fA-F]{1,4}){1,6})|:((:[0-9a-fA-F]{1,4}){1,7}|:)|fe80:(:[0-9a-fA-F]{0,4}){0,4}%[0-9a-zA-Z]{1,}|::(ffff(:0{1,4}){0,1}:){0,1}((25[0-5]|(2[0-4]|1{0,1}[0-9]){0,1}[0-9])\\.){3,3}(25[0-5]|(2[0-4]|1{0,1}[0-9]){0,1}[0-9])|([0-9a-fA-F]{1,4}:){1,4}:((25[0-5]|(2[0-4]|1{0,1}[0-9]){0,1}[0-9])\\.){3,3}(25[0-5]|(2[0-4]|1{0,1}[0-9]){0,1}[0-9]))$/, pd = /^(([0-9a-fA-F]{1,4}:){7,7}[0-9a-fA-F]{1,4}|([0-9a-fA-F]{1,4}:){1,7}:|([0-9a-fA-F]{1,4}:){1,6}:[0-9a-fA-F]{1,4}|([0-9a-fA-F]{1,4}:){1,5}(:[0-9a-fA-F]{1,4}){1,2}|([0-9a-fA-F]{1,4}:){1,4}(:[0-9a-fA-F]{1,4}){1,3}|([0-9a-fA-F]{1,4}:){1,3}(:[0-9a-fA-F]{1,4}){1,4}|([0-9a-fA-F]{1,4}:){1,2}(:[0-9a-fA-F]{1,4}){1,5}|[0-9a-fA-F]{1,4}:((:[0-9a-fA-F]{1,4}){1,6})|:((:[0-9a-fA-F]{1,4}){1,7}|:)|fe80:(:[0-9a-fA-F]{0,4}){0,4}%[0-9a-zA-Z]{1,}|::(ffff(:0{1,4}){0,1}:){0,1}((25[0-5]|(2[0-4]|1{0,1}[0-9]){0,1}[0-9])\\.){3,3}(25[0-5]|(2[0-4]|1{0,1}[0-9]){0,1}[0-9])|([0-9a-fA-F]{1,4}:){1,4}:((25[0-5]|(2[0-4]|1{0,1}[0-9]){0,1}[0-9])\\.){3,3}(25[0-5]|(2[0-4]|1{0,1}[0-9]){0,1}[0-9]))\\/(12[0-8]|1[01][0-9]|[1-9]?[0-9])$/, fd = /^([0-9a-zA-Z+/]{4})*(([0-9a-zA-Z+/]{2}==)|([0-9a-zA-Z+/]{3}=))?$/, yd = /^([0-9a-zA-Z-_]{4})*(([0-9a-zA-Z-_]{2}(==)?)|([0-9a-zA-Z-_]{3}(=)?))?$/, Zt = "((\\\\d\\\\d[2468][048]|\\\\d\\\\d[13579][26]|\\\\d\\\\d0[48]|[02468][048]00|[13579][26]00)-02-29|\\\\d{4}-((0[13578]|1[02])-(0[1-9]|[12]\\\\d|3[01])|(0[469]|11)-(0[1-9]|[12]\\\\d|30)|(02)-(0[1-9]|1\\\\d|2[0-8])))", _d = new RegExp(\`^\${Zt}$\`);
	function qt(e) {
		let t = "[0-5]\\\\d";
		e.precision ? t = \`\${t}\\\\.\\\\d{\${e.precision}}\` : e.precision == null && (t = \`\${t}(\\\\.\\\\d+)?\`);
		const a = e.precision ? "+" : "?";
		return \`([01]\\\\d|2[0-3]):[0-5]\\\\d(:\${t})\${a}\`;
	}
	function bd(e) {
		return new RegExp(\`^\${qt(e)}$\`);
	}
	function gd(e) {
		let t = \`\${Zt}T\${qt(e)}\`;
		const a = [];
		return a.push(e.local ? "Z?" : "Z"), e.offset && a.push("([+-]\\\\d{2}:?\\\\d{2})"), t = \`\${t}(\${a.join("|")})\`, new RegExp(\`^\${t}$\`);
	}
	function wd(e, t) {
		return !!((t === "v4" || !t) && ud.test(e) || (t === "v6" || !t) && md.test(e));
	}
	function vd(e, t) {
		if (!od.test(e)) return !1;
		try {
			const [a] = e.split(".");
			if (!a) return !1;
			const r = a.replace(/-/g, "+").replace(/_/g, "/").padEnd(a.length + (4 - a.length % 4) % 4, "="), s = JSON.parse(atob(r));
			return !(typeof s != "object" || s === null || "typ" in s && s?.typ !== "JWT" || !s.alg || t && s.alg !== t);
		} catch {
			return !1;
		}
	}
	function Sd(e, t) {
		return !!((t === "v4" || !t) && hd.test(e) || (t === "v6" || !t) && pd.test(e));
	}
	var fe = class me extends T {
		_parse(t) {
			if (this._def.coerce && (t.data = String(t.data)), this._getType(t) !== _.string) {
				const s = this._getOrReturnCtx(t);
				return y(s, {
					code: m.invalid_type,
					expected: _.string,
					received: s.parsedType
				}), w;
			}
			const a = new N();
			let r;
			for (const s of this._def.checks) if (s.kind === "min") t.data.length < s.value && (r = this._getOrReturnCtx(t, r), y(r, {
				code: m.too_small,
				minimum: s.value,
				type: "string",
				inclusive: !0,
				exact: !1,
				message: s.message
			}), a.dirty());
			else if (s.kind === "max") t.data.length > s.value && (r = this._getOrReturnCtx(t, r), y(r, {
				code: m.too_big,
				maximum: s.value,
				type: "string",
				inclusive: !0,
				exact: !1,
				message: s.message
			}), a.dirty());
			else if (s.kind === "length") {
				const c = t.data.length > s.value, o = t.data.length < s.value;
				(c || o) && (r = this._getOrReturnCtx(t, r), c ? y(r, {
					code: m.too_big,
					maximum: s.value,
					type: "string",
					inclusive: !0,
					exact: !0,
					message: s.message
				}) : o && y(r, {
					code: m.too_small,
					minimum: s.value,
					type: "string",
					inclusive: !0,
					exact: !0,
					message: s.message
				}), a.dirty());
			} else if (s.kind === "email") dd.test(t.data) || (r = this._getOrReturnCtx(t, r), y(r, {
				validation: "email",
				code: m.invalid_string,
				message: s.message
			}), a.dirty());
			else if (s.kind === "emoji") Fe || (Fe = new RegExp(ld, "u")), Fe.test(t.data) || (r = this._getOrReturnCtx(t, r), y(r, {
				validation: "emoji",
				code: m.invalid_string,
				message: s.message
			}), a.dirty());
			else if (s.kind === "uuid") id.test(t.data) || (r = this._getOrReturnCtx(t, r), y(r, {
				validation: "uuid",
				code: m.invalid_string,
				message: s.message
			}), a.dirty());
			else if (s.kind === "nanoid") nd.test(t.data) || (r = this._getOrReturnCtx(t, r), y(r, {
				validation: "nanoid",
				code: m.invalid_string,
				message: s.message
			}), a.dirty());
			else if (s.kind === "cuid") ad.test(t.data) || (r = this._getOrReturnCtx(t, r), y(r, {
				validation: "cuid",
				code: m.invalid_string,
				message: s.message
			}), a.dirty());
			else if (s.kind === "cuid2") rd.test(t.data) || (r = this._getOrReturnCtx(t, r), y(r, {
				validation: "cuid2",
				code: m.invalid_string,
				message: s.message
			}), a.dirty());
			else if (s.kind === "ulid") sd.test(t.data) || (r = this._getOrReturnCtx(t, r), y(r, {
				validation: "ulid",
				code: m.invalid_string,
				message: s.message
			}), a.dirty());
			else if (s.kind === "url") try {
				new URL(t.data);
			} catch {
				r = this._getOrReturnCtx(t, r), y(r, {
					validation: "url",
					code: m.invalid_string,
					message: s.message
				}), a.dirty();
			}
			else s.kind === "regex" ? (s.regex.lastIndex = 0, s.regex.test(t.data) || (r = this._getOrReturnCtx(t, r), y(r, {
				validation: "regex",
				code: m.invalid_string,
				message: s.message
			}), a.dirty())) : s.kind === "trim" ? t.data = t.data.trim() : s.kind === "includes" ? t.data.includes(s.value, s.position) || (r = this._getOrReturnCtx(t, r), y(r, {
				code: m.invalid_string,
				validation: {
					includes: s.value,
					position: s.position
				},
				message: s.message
			}), a.dirty()) : s.kind === "toLowerCase" ? t.data = t.data.toLowerCase() : s.kind === "toUpperCase" ? t.data = t.data.toUpperCase() : s.kind === "startsWith" ? t.data.startsWith(s.value) || (r = this._getOrReturnCtx(t, r), y(r, {
				code: m.invalid_string,
				validation: { startsWith: s.value },
				message: s.message
			}), a.dirty()) : s.kind === "endsWith" ? t.data.endsWith(s.value) || (r = this._getOrReturnCtx(t, r), y(r, {
				code: m.invalid_string,
				validation: { endsWith: s.value },
				message: s.message
			}), a.dirty()) : s.kind === "datetime" ? gd(s).test(t.data) || (r = this._getOrReturnCtx(t, r), y(r, {
				code: m.invalid_string,
				validation: "datetime",
				message: s.message
			}), a.dirty()) : s.kind === "date" ? _d.test(t.data) || (r = this._getOrReturnCtx(t, r), y(r, {
				code: m.invalid_string,
				validation: "date",
				message: s.message
			}), a.dirty()) : s.kind === "time" ? bd(s).test(t.data) || (r = this._getOrReturnCtx(t, r), y(r, {
				code: m.invalid_string,
				validation: "time",
				message: s.message
			}), a.dirty()) : s.kind === "duration" ? cd.test(t.data) || (r = this._getOrReturnCtx(t, r), y(r, {
				validation: "duration",
				code: m.invalid_string,
				message: s.message
			}), a.dirty()) : s.kind === "ip" ? wd(t.data, s.version) || (r = this._getOrReturnCtx(t, r), y(r, {
				validation: "ip",
				code: m.invalid_string,
				message: s.message
			}), a.dirty()) : s.kind === "jwt" ? vd(t.data, s.alg) || (r = this._getOrReturnCtx(t, r), y(r, {
				validation: "jwt",
				code: m.invalid_string,
				message: s.message
			}), a.dirty()) : s.kind === "cidr" ? Sd(t.data, s.version) || (r = this._getOrReturnCtx(t, r), y(r, {
				validation: "cidr",
				code: m.invalid_string,
				message: s.message
			}), a.dirty()) : s.kind === "base64" ? fd.test(t.data) || (r = this._getOrReturnCtx(t, r), y(r, {
				validation: "base64",
				code: m.invalid_string,
				message: s.message
			}), a.dirty()) : s.kind === "base64url" ? yd.test(t.data) || (r = this._getOrReturnCtx(t, r), y(r, {
				validation: "base64url",
				code: m.invalid_string,
				message: s.message
			}), a.dirty()) : $.assertNever(s);
			return {
				status: a.value,
				value: t.data
			};
		}
		_regex(t, a, r) {
			return this.refinement((s) => t.test(s), {
				validation: a,
				code: m.invalid_string,
				...b.errToObj(r)
			});
		}
		_addCheck(t) {
			return new me({
				...this._def,
				checks: [...this._def.checks, t]
			});
		}
		email(t) {
			return this._addCheck({
				kind: "email",
				...b.errToObj(t)
			});
		}
		url(t) {
			return this._addCheck({
				kind: "url",
				...b.errToObj(t)
			});
		}
		emoji(t) {
			return this._addCheck({
				kind: "emoji",
				...b.errToObj(t)
			});
		}
		uuid(t) {
			return this._addCheck({
				kind: "uuid",
				...b.errToObj(t)
			});
		}
		nanoid(t) {
			return this._addCheck({
				kind: "nanoid",
				...b.errToObj(t)
			});
		}
		cuid(t) {
			return this._addCheck({
				kind: "cuid",
				...b.errToObj(t)
			});
		}
		cuid2(t) {
			return this._addCheck({
				kind: "cuid2",
				...b.errToObj(t)
			});
		}
		ulid(t) {
			return this._addCheck({
				kind: "ulid",
				...b.errToObj(t)
			});
		}
		base64(t) {
			return this._addCheck({
				kind: "base64",
				...b.errToObj(t)
			});
		}
		base64url(t) {
			return this._addCheck({
				kind: "base64url",
				...b.errToObj(t)
			});
		}
		jwt(t) {
			return this._addCheck({
				kind: "jwt",
				...b.errToObj(t)
			});
		}
		ip(t) {
			return this._addCheck({
				kind: "ip",
				...b.errToObj(t)
			});
		}
		cidr(t) {
			return this._addCheck({
				kind: "cidr",
				...b.errToObj(t)
			});
		}
		datetime(t) {
			return typeof t == "string" ? this._addCheck({
				kind: "datetime",
				precision: null,
				offset: !1,
				local: !1,
				message: t
			}) : this._addCheck({
				kind: "datetime",
				precision: typeof t?.precision > "u" ? null : t?.precision,
				offset: t?.offset ?? !1,
				local: t?.local ?? !1,
				...b.errToObj(t?.message)
			});
		}
		date(t) {
			return this._addCheck({
				kind: "date",
				message: t
			});
		}
		time(t) {
			return typeof t == "string" ? this._addCheck({
				kind: "time",
				precision: null,
				message: t
			}) : this._addCheck({
				kind: "time",
				precision: typeof t?.precision > "u" ? null : t?.precision,
				...b.errToObj(t?.message)
			});
		}
		duration(t) {
			return this._addCheck({
				kind: "duration",
				...b.errToObj(t)
			});
		}
		regex(t, a) {
			return this._addCheck({
				kind: "regex",
				regex: t,
				...b.errToObj(a)
			});
		}
		includes(t, a) {
			return this._addCheck({
				kind: "includes",
				value: t,
				position: a?.position,
				...b.errToObj(a?.message)
			});
		}
		startsWith(t, a) {
			return this._addCheck({
				kind: "startsWith",
				value: t,
				...b.errToObj(a)
			});
		}
		endsWith(t, a) {
			return this._addCheck({
				kind: "endsWith",
				value: t,
				...b.errToObj(a)
			});
		}
		min(t, a) {
			return this._addCheck({
				kind: "min",
				value: t,
				...b.errToObj(a)
			});
		}
		max(t, a) {
			return this._addCheck({
				kind: "max",
				value: t,
				...b.errToObj(a)
			});
		}
		length(t, a) {
			return this._addCheck({
				kind: "length",
				value: t,
				...b.errToObj(a)
			});
		}
		nonempty(t) {
			return this.min(1, b.errToObj(t));
		}
		trim() {
			return new me({
				...this._def,
				checks: [...this._def.checks, { kind: "trim" }]
			});
		}
		toLowerCase() {
			return new me({
				...this._def,
				checks: [...this._def.checks, { kind: "toLowerCase" }]
			});
		}
		toUpperCase() {
			return new me({
				...this._def,
				checks: [...this._def.checks, { kind: "toUpperCase" }]
			});
		}
		get isDatetime() {
			return !!this._def.checks.find((t) => t.kind === "datetime");
		}
		get isDate() {
			return !!this._def.checks.find((t) => t.kind === "date");
		}
		get isTime() {
			return !!this._def.checks.find((t) => t.kind === "time");
		}
		get isDuration() {
			return !!this._def.checks.find((t) => t.kind === "duration");
		}
		get isEmail() {
			return !!this._def.checks.find((t) => t.kind === "email");
		}
		get isURL() {
			return !!this._def.checks.find((t) => t.kind === "url");
		}
		get isEmoji() {
			return !!this._def.checks.find((t) => t.kind === "emoji");
		}
		get isUUID() {
			return !!this._def.checks.find((t) => t.kind === "uuid");
		}
		get isNANOID() {
			return !!this._def.checks.find((t) => t.kind === "nanoid");
		}
		get isCUID() {
			return !!this._def.checks.find((t) => t.kind === "cuid");
		}
		get isCUID2() {
			return !!this._def.checks.find((t) => t.kind === "cuid2");
		}
		get isULID() {
			return !!this._def.checks.find((t) => t.kind === "ulid");
		}
		get isIP() {
			return !!this._def.checks.find((t) => t.kind === "ip");
		}
		get isCIDR() {
			return !!this._def.checks.find((t) => t.kind === "cidr");
		}
		get isBase64() {
			return !!this._def.checks.find((t) => t.kind === "base64");
		}
		get isBase64url() {
			return !!this._def.checks.find((t) => t.kind === "base64url");
		}
		get minLength() {
			let t = null;
			for (const a of this._def.checks) a.kind === "min" && (t === null || a.value > t) && (t = a.value);
			return t;
		}
		get maxLength() {
			let t = null;
			for (const a of this._def.checks) a.kind === "max" && (t === null || a.value < t) && (t = a.value);
			return t;
		}
	};
	fe.create = (e) => new fe({
		checks: [],
		typeName: v.ZodString,
		coerce: e?.coerce ?? !1,
		...x(e)
	});
	function xd(e, t) {
		const a = (e.toString().split(".")[1] || "").length, r = (t.toString().split(".")[1] || "").length, s = a > r ? a : r;
		return Number.parseInt(e.toFixed(s).replace(".", "")) % Number.parseInt(t.toFixed(s).replace(".", "")) / 10 ** s;
	}
	var Ve = class It extends T {
		constructor() {
			super(...arguments), this.min = this.gte, this.max = this.lte, this.step = this.multipleOf;
		}
		_parse(t) {
			if (this._def.coerce && (t.data = Number(t.data)), this._getType(t) !== _.number) {
				const s = this._getOrReturnCtx(t);
				return y(s, {
					code: m.invalid_type,
					expected: _.number,
					received: s.parsedType
				}), w;
			}
			let a;
			const r = new N();
			for (const s of this._def.checks) s.kind === "int" ? $.isInteger(t.data) || (a = this._getOrReturnCtx(t, a), y(a, {
				code: m.invalid_type,
				expected: "integer",
				received: "float",
				message: s.message
			}), r.dirty()) : s.kind === "min" ? (s.inclusive ? t.data < s.value : t.data <= s.value) && (a = this._getOrReturnCtx(t, a), y(a, {
				code: m.too_small,
				minimum: s.value,
				type: "number",
				inclusive: s.inclusive,
				exact: !1,
				message: s.message
			}), r.dirty()) : s.kind === "max" ? (s.inclusive ? t.data > s.value : t.data >= s.value) && (a = this._getOrReturnCtx(t, a), y(a, {
				code: m.too_big,
				maximum: s.value,
				type: "number",
				inclusive: s.inclusive,
				exact: !1,
				message: s.message
			}), r.dirty()) : s.kind === "multipleOf" ? xd(t.data, s.value) !== 0 && (a = this._getOrReturnCtx(t, a), y(a, {
				code: m.not_multiple_of,
				multipleOf: s.value,
				message: s.message
			}), r.dirty()) : s.kind === "finite" ? Number.isFinite(t.data) || (a = this._getOrReturnCtx(t, a), y(a, {
				code: m.not_finite,
				message: s.message
			}), r.dirty()) : $.assertNever(s);
			return {
				status: r.value,
				value: t.data
			};
		}
		gte(t, a) {
			return this.setLimit("min", t, !0, b.toString(a));
		}
		gt(t, a) {
			return this.setLimit("min", t, !1, b.toString(a));
		}
		lte(t, a) {
			return this.setLimit("max", t, !0, b.toString(a));
		}
		lt(t, a) {
			return this.setLimit("max", t, !1, b.toString(a));
		}
		setLimit(t, a, r, s) {
			return new It({
				...this._def,
				checks: [...this._def.checks, {
					kind: t,
					value: a,
					inclusive: r,
					message: b.toString(s)
				}]
			});
		}
		_addCheck(t) {
			return new It({
				...this._def,
				checks: [...this._def.checks, t]
			});
		}
		int(t) {
			return this._addCheck({
				kind: "int",
				message: b.toString(t)
			});
		}
		positive(t) {
			return this._addCheck({
				kind: "min",
				value: 0,
				inclusive: !1,
				message: b.toString(t)
			});
		}
		negative(t) {
			return this._addCheck({
				kind: "max",
				value: 0,
				inclusive: !1,
				message: b.toString(t)
			});
		}
		nonpositive(t) {
			return this._addCheck({
				kind: "max",
				value: 0,
				inclusive: !0,
				message: b.toString(t)
			});
		}
		nonnegative(t) {
			return this._addCheck({
				kind: "min",
				value: 0,
				inclusive: !0,
				message: b.toString(t)
			});
		}
		multipleOf(t, a) {
			return this._addCheck({
				kind: "multipleOf",
				value: t,
				message: b.toString(a)
			});
		}
		finite(t) {
			return this._addCheck({
				kind: "finite",
				message: b.toString(t)
			});
		}
		safe(t) {
			return this._addCheck({
				kind: "min",
				inclusive: !0,
				value: Number.MIN_SAFE_INTEGER,
				message: b.toString(t)
			})._addCheck({
				kind: "max",
				inclusive: !0,
				value: Number.MAX_SAFE_INTEGER,
				message: b.toString(t)
			});
		}
		get minValue() {
			let t = null;
			for (const a of this._def.checks) a.kind === "min" && (t === null || a.value > t) && (t = a.value);
			return t;
		}
		get maxValue() {
			let t = null;
			for (const a of this._def.checks) a.kind === "max" && (t === null || a.value < t) && (t = a.value);
			return t;
		}
		get isInt() {
			return !!this._def.checks.find((t) => t.kind === "int" || t.kind === "multipleOf" && $.isInteger(t.value));
		}
		get isFinite() {
			let t = null, a = null;
			for (const r of this._def.checks) {
				if (r.kind === "finite" || r.kind === "int" || r.kind === "multipleOf") return !0;
				r.kind === "min" ? (a === null || r.value > a) && (a = r.value) : r.kind === "max" && (t === null || r.value < t) && (t = r.value);
			}
			return Number.isFinite(a) && Number.isFinite(t);
		}
	};
	Ve.create = (e) => new Ve({
		checks: [],
		typeName: v.ZodNumber,
		coerce: e?.coerce || !1,
		...x(e)
	});
	var He = class Pt extends T {
		constructor() {
			super(...arguments), this.min = this.gte, this.max = this.lte;
		}
		_parse(t) {
			if (this._def.coerce) try {
				t.data = BigInt(t.data);
			} catch {
				return this._getInvalidInput(t);
			}
			if (this._getType(t) !== _.bigint) return this._getInvalidInput(t);
			let a;
			const r = new N();
			for (const s of this._def.checks) s.kind === "min" ? (s.inclusive ? t.data < s.value : t.data <= s.value) && (a = this._getOrReturnCtx(t, a), y(a, {
				code: m.too_small,
				type: "bigint",
				minimum: s.value,
				inclusive: s.inclusive,
				message: s.message
			}), r.dirty()) : s.kind === "max" ? (s.inclusive ? t.data > s.value : t.data >= s.value) && (a = this._getOrReturnCtx(t, a), y(a, {
				code: m.too_big,
				type: "bigint",
				maximum: s.value,
				inclusive: s.inclusive,
				message: s.message
			}), r.dirty()) : s.kind === "multipleOf" ? t.data % s.value !== BigInt(0) && (a = this._getOrReturnCtx(t, a), y(a, {
				code: m.not_multiple_of,
				multipleOf: s.value,
				message: s.message
			}), r.dirty()) : $.assertNever(s);
			return {
				status: r.value,
				value: t.data
			};
		}
		_getInvalidInput(t) {
			const a = this._getOrReturnCtx(t);
			return y(a, {
				code: m.invalid_type,
				expected: _.bigint,
				received: a.parsedType
			}), w;
		}
		gte(t, a) {
			return this.setLimit("min", t, !0, b.toString(a));
		}
		gt(t, a) {
			return this.setLimit("min", t, !1, b.toString(a));
		}
		lte(t, a) {
			return this.setLimit("max", t, !0, b.toString(a));
		}
		lt(t, a) {
			return this.setLimit("max", t, !1, b.toString(a));
		}
		setLimit(t, a, r, s) {
			return new Pt({
				...this._def,
				checks: [...this._def.checks, {
					kind: t,
					value: a,
					inclusive: r,
					message: b.toString(s)
				}]
			});
		}
		_addCheck(t) {
			return new Pt({
				...this._def,
				checks: [...this._def.checks, t]
			});
		}
		positive(t) {
			return this._addCheck({
				kind: "min",
				value: BigInt(0),
				inclusive: !1,
				message: b.toString(t)
			});
		}
		negative(t) {
			return this._addCheck({
				kind: "max",
				value: BigInt(0),
				inclusive: !1,
				message: b.toString(t)
			});
		}
		nonpositive(t) {
			return this._addCheck({
				kind: "max",
				value: BigInt(0),
				inclusive: !0,
				message: b.toString(t)
			});
		}
		nonnegative(t) {
			return this._addCheck({
				kind: "min",
				value: BigInt(0),
				inclusive: !0,
				message: b.toString(t)
			});
		}
		multipleOf(t, a) {
			return this._addCheck({
				kind: "multipleOf",
				value: t,
				message: b.toString(a)
			});
		}
		get minValue() {
			let t = null;
			for (const a of this._def.checks) a.kind === "min" && (t === null || a.value > t) && (t = a.value);
			return t;
		}
		get maxValue() {
			let t = null;
			for (const a of this._def.checks) a.kind === "max" && (t === null || a.value < t) && (t = a.value);
			return t;
		}
	};
	He.create = (e) => new He({
		checks: [],
		typeName: v.ZodBigInt,
		coerce: e?.coerce ?? !1,
		...x(e)
	});
	var We = class extends T {
		_parse(e) {
			if (this._def.coerce && (e.data = !!e.data), this._getType(e) !== _.boolean) {
				const t = this._getOrReturnCtx(e);
				return y(t, {
					code: m.invalid_type,
					expected: _.boolean,
					received: t.parsedType
				}), w;
			}
			return E(e.data);
		}
	};
	We.create = (e) => new We({
		typeName: v.ZodBoolean,
		coerce: e?.coerce || !1,
		...x(e)
	});
	var ze = class Hc extends T {
		_parse(t) {
			if (this._def.coerce && (t.data = new Date(t.data)), this._getType(t) !== _.date) {
				const s = this._getOrReturnCtx(t);
				return y(s, {
					code: m.invalid_type,
					expected: _.date,
					received: s.parsedType
				}), w;
			}
			if (Number.isNaN(t.data.getTime())) return y(this._getOrReturnCtx(t), { code: m.invalid_date }), w;
			const a = new N();
			let r;
			for (const s of this._def.checks) s.kind === "min" ? t.data.getTime() < s.value && (r = this._getOrReturnCtx(t, r), y(r, {
				code: m.too_small,
				message: s.message,
				inclusive: !0,
				exact: !1,
				minimum: s.value,
				type: "date"
			}), a.dirty()) : s.kind === "max" ? t.data.getTime() > s.value && (r = this._getOrReturnCtx(t, r), y(r, {
				code: m.too_big,
				message: s.message,
				inclusive: !0,
				exact: !1,
				maximum: s.value,
				type: "date"
			}), a.dirty()) : $.assertNever(s);
			return {
				status: a.value,
				value: new Date(t.data.getTime())
			};
		}
		_addCheck(t) {
			return new Hc({
				...this._def,
				checks: [...this._def.checks, t]
			});
		}
		min(t, a) {
			return this._addCheck({
				kind: "min",
				value: t.getTime(),
				message: b.toString(a)
			});
		}
		max(t, a) {
			return this._addCheck({
				kind: "max",
				value: t.getTime(),
				message: b.toString(a)
			});
		}
		get minDate() {
			let t = null;
			for (const a of this._def.checks) a.kind === "min" && (t === null || a.value > t) && (t = a.value);
			return t != null ? new Date(t) : null;
		}
		get maxDate() {
			let t = null;
			for (const a of this._def.checks) a.kind === "max" && (t === null || a.value < t) && (t = a.value);
			return t != null ? new Date(t) : null;
		}
	};
	ze.create = (e) => new ze({
		checks: [],
		coerce: e?.coerce || !1,
		typeName: v.ZodDate,
		...x(e)
	});
	var Ge = class extends T {
		_parse(e) {
			if (this._getType(e) !== _.symbol) {
				const t = this._getOrReturnCtx(e);
				return y(t, {
					code: m.invalid_type,
					expected: _.symbol,
					received: t.parsedType
				}), w;
			}
			return E(e.data);
		}
	};
	Ge.create = (e) => new Ge({
		typeName: v.ZodSymbol,
		...x(e)
	});
	var ye = class extends T {
		_parse(e) {
			if (this._getType(e) !== _.undefined) {
				const t = this._getOrReturnCtx(e);
				return y(t, {
					code: m.invalid_type,
					expected: _.undefined,
					received: t.parsedType
				}), w;
			}
			return E(e.data);
		}
	};
	ye.create = (e) => new ye({
		typeName: v.ZodUndefined,
		...x(e)
	});
	var _e = class extends T {
		_parse(e) {
			if (this._getType(e) !== _.null) {
				const t = this._getOrReturnCtx(e);
				return y(t, {
					code: m.invalid_type,
					expected: _.null,
					received: t.parsedType
				}), w;
			}
			return E(e.data);
		}
	};
	_e.create = (e) => new _e({
		typeName: v.ZodNull,
		...x(e)
	});
	var Je = class extends T {
		constructor() {
			super(...arguments), this._any = !0;
		}
		_parse(e) {
			return E(e.data);
		}
	};
	Je.create = (e) => new Je({
		typeName: v.ZodAny,
		...x(e)
	});
	var X = class extends T {
		constructor() {
			super(...arguments), this._unknown = !0;
		}
		_parse(e) {
			return E(e.data);
		}
	};
	X.create = (e) => new X({
		typeName: v.ZodUnknown,
		...x(e)
	});
	var z = class extends T {
		_parse(e) {
			const t = this._getOrReturnCtx(e);
			return y(t, {
				code: m.invalid_type,
				expected: _.never,
				received: t.parsedType
			}), w;
		}
	};
	z.create = (e) => new z({
		typeName: v.ZodNever,
		...x(e)
	});
	var Ke = class extends T {
		_parse(e) {
			if (this._getType(e) !== _.undefined) {
				const t = this._getOrReturnCtx(e);
				return y(t, {
					code: m.invalid_type,
					expected: _.void,
					received: t.parsedType
				}), w;
			}
			return E(e.data);
		}
	};
	Ke.create = (e) => new Ke({
		typeName: v.ZodVoid,
		...x(e)
	});
	var Q = class Le extends T {
		_parse(t) {
			const { ctx: a, status: r } = this._processInputParams(t), s = this._def;
			if (a.parsedType !== _.array) return y(a, {
				code: m.invalid_type,
				expected: _.array,
				received: a.parsedType
			}), w;
			if (s.exactLength !== null) {
				const o = a.data.length > s.exactLength.value, d = a.data.length < s.exactLength.value;
				(o || d) && (y(a, {
					code: o ? m.too_big : m.too_small,
					minimum: d ? s.exactLength.value : void 0,
					maximum: o ? s.exactLength.value : void 0,
					type: "array",
					inclusive: !0,
					exact: !0,
					message: s.exactLength.message
				}), r.dirty());
			}
			if (s.minLength !== null && a.data.length < s.minLength.value && (y(a, {
				code: m.too_small,
				minimum: s.minLength.value,
				type: "array",
				inclusive: !0,
				exact: !1,
				message: s.minLength.message
			}), r.dirty()), s.maxLength !== null && a.data.length > s.maxLength.value && (y(a, {
				code: m.too_big,
				maximum: s.maxLength.value,
				type: "array",
				inclusive: !0,
				exact: !1,
				message: s.maxLength.message
			}), r.dirty()), a.common.async) return Promise.all([...a.data].map((o, d) => s.type._parseAsync(new L(a, o, a.path, d)))).then((o) => N.mergeArray(r, o));
			const c = [...a.data].map((o, d) => s.type._parseSync(new L(a, o, a.path, d)));
			return N.mergeArray(r, c);
		}
		get element() {
			return this._def.type;
		}
		min(t, a) {
			return new Le({
				...this._def,
				minLength: {
					value: t,
					message: b.toString(a)
				}
			});
		}
		max(t, a) {
			return new Le({
				...this._def,
				maxLength: {
					value: t,
					message: b.toString(a)
				}
			});
		}
		length(t, a) {
			return new Le({
				...this._def,
				exactLength: {
					value: t,
					message: b.toString(a)
				}
			});
		}
		nonempty(t) {
			return this.min(1, t);
		}
	};
	Q.create = (e, t) => new Q({
		type: e,
		minLength: null,
		maxLength: null,
		exactLength: null,
		typeName: v.ZodArray,
		...x(t)
	});
	function ee(e) {
		if (e instanceof O) {
			const t = {};
			for (const a in e.shape) {
				const r = e.shape[a];
				t[a] = H.create(ee(r));
			}
			return new O({
				...e._def,
				shape: () => t
			});
		} else return e instanceof Q ? new Q({
			...e._def,
			type: ee(e.element)
		}) : e instanceof H ? H.create(ee(e.unwrap())) : e instanceof J ? J.create(ee(e.unwrap())) : e instanceof G ? G.create(e.items.map((t) => ee(t))) : e;
	}
	var O = class B extends T {
		constructor() {
			super(...arguments), this._cached = null, this.nonstrict = this.passthrough, this.augment = this.extend;
		}
		_getCached() {
			if (this._cached !== null) return this._cached;
			const t = this._def.shape(), a = $.objectKeys(t);
			return this._cached = {
				shape: t,
				keys: a
			}, this._cached;
		}
		_parse(t) {
			if (this._getType(t) !== _.object) {
				const p = this._getOrReturnCtx(t);
				return y(p, {
					code: m.invalid_type,
					expected: _.object,
					received: p.parsedType
				}), w;
			}
			const { status: a, ctx: r } = this._processInputParams(t), { shape: s, keys: c } = this._getCached(), o = [];
			if (!(this._def.catchall instanceof z && this._def.unknownKeys === "strip")) for (const p in r.data) c.includes(p) || o.push(p);
			const d = [];
			for (const p of c) {
				const f = s[p], R = r.data[p];
				d.push({
					key: {
						status: "valid",
						value: p
					},
					value: f._parse(new L(r, R, r.path, p)),
					alwaysSet: p in r.data
				});
			}
			if (this._def.catchall instanceof z) {
				const p = this._def.unknownKeys;
				if (p === "passthrough") for (const f of o) d.push({
					key: {
						status: "valid",
						value: f
					},
					value: {
						status: "valid",
						value: r.data[f]
					}
				});
				else if (p === "strict") o.length > 0 && (y(r, {
					code: m.unrecognized_keys,
					keys: o
				}), a.dirty());
				else if (p !== "strip") throw new Error("Internal ZodObject error: invalid unknownKeys value.");
			} else {
				const p = this._def.catchall;
				for (const f of o) {
					const R = r.data[f];
					d.push({
						key: {
							status: "valid",
							value: f
						},
						value: p._parse(new L(r, R, r.path, f)),
						alwaysSet: f in r.data
					});
				}
			}
			return r.common.async ? Promise.resolve().then(async () => {
				const p = [];
				for (const f of d) {
					const R = await f.key, P = await f.value;
					p.push({
						key: R,
						value: P,
						alwaysSet: f.alwaysSet
					});
				}
				return p;
			}).then((p) => N.mergeObjectSync(a, p)) : N.mergeObjectSync(a, d);
		}
		get shape() {
			return this._def.shape();
		}
		strict(t) {
			return b.errToObj, new B({
				...this._def,
				unknownKeys: "strict",
				...t !== void 0 ? { errorMap: (a, r) => {
					const s = this._def.errorMap?.(a, r).message ?? r.defaultError;
					return a.code === "unrecognized_keys" ? { message: b.errToObj(t).message ?? s } : { message: s };
				} } : {}
			});
		}
		strip() {
			return new B({
				...this._def,
				unknownKeys: "strip"
			});
		}
		passthrough() {
			return new B({
				...this._def,
				unknownKeys: "passthrough"
			});
		}
		extend(t) {
			return new B({
				...this._def,
				shape: () => ({
					...this._def.shape(),
					...t
				})
			});
		}
		merge(t) {
			return new B({
				unknownKeys: t._def.unknownKeys,
				catchall: t._def.catchall,
				shape: () => ({
					...this._def.shape(),
					...t._def.shape()
				}),
				typeName: v.ZodObject
			});
		}
		setKey(t, a) {
			return this.augment({ [t]: a });
		}
		catchall(t) {
			return new B({
				...this._def,
				catchall: t
			});
		}
		pick(t) {
			const a = {};
			for (const r of $.objectKeys(t)) t[r] && this.shape[r] && (a[r] = this.shape[r]);
			return new B({
				...this._def,
				shape: () => a
			});
		}
		omit(t) {
			const a = {};
			for (const r of $.objectKeys(this.shape)) t[r] || (a[r] = this.shape[r]);
			return new B({
				...this._def,
				shape: () => a
			});
		}
		deepPartial() {
			return ee(this);
		}
		partial(t) {
			const a = {};
			for (const r of $.objectKeys(this.shape)) {
				const s = this.shape[r];
				t && !t[r] ? a[r] = s : a[r] = s.optional();
			}
			return new B({
				...this._def,
				shape: () => a
			});
		}
		required(t) {
			const a = {};
			for (const r of $.objectKeys(this.shape)) if (t && !t[r]) a[r] = this.shape[r];
			else {
				let s = this.shape[r];
				for (; s instanceof H;) s = s._def.innerType;
				a[r] = s;
			}
			return new B({
				...this._def,
				shape: () => a
			});
		}
		keyof() {
			return Ut($.objectKeys(this.shape));
		}
	};
	O.create = (e, t) => new O({
		shape: () => e,
		unknownKeys: "strip",
		catchall: z.create(),
		typeName: v.ZodObject,
		...x(t)
	}), O.strictCreate = (e, t) => new O({
		shape: () => e,
		unknownKeys: "strict",
		catchall: z.create(),
		typeName: v.ZodObject,
		...x(t)
	}), O.lazycreate = (e, t) => new O({
		shape: e,
		unknownKeys: "strip",
		catchall: z.create(),
		typeName: v.ZodObject,
		...x(t)
	});
	var be = class extends T {
		_parse(e) {
			const { ctx: t } = this._processInputParams(e), a = this._def.options;
			function r(s) {
				for (const o of s) if (o.result.status === "valid") return o.result;
				for (const o of s) if (o.result.status === "dirty") return t.common.issues.push(...o.ctx.common.issues), o.result;
				const c = s.map((o) => new j(o.ctx.common.issues));
				return y(t, {
					code: m.invalid_union,
					unionErrors: c
				}), w;
			}
			if (t.common.async) return Promise.all(a.map(async (s) => {
				const c = {
					...t,
					common: {
						...t.common,
						issues: []
					},
					parent: null
				};
				return {
					result: await s._parseAsync({
						data: t.data,
						path: t.path,
						parent: c
					}),
					ctx: c
				};
			})).then(r);
			{
				let s;
				const c = [];
				for (const d of a) {
					const p = {
						...t,
						common: {
							...t.common,
							issues: []
						},
						parent: null
					}, f = d._parseSync({
						data: t.data,
						path: t.path,
						parent: p
					});
					if (f.status === "valid") return f;
					f.status === "dirty" && !s && (s = {
						result: f,
						ctx: p
					}), p.common.issues.length && c.push(p.common.issues);
				}
				if (s) return t.common.issues.push(...s.ctx.common.issues), s.result;
				const o = c.map((d) => new j(d));
				return y(t, {
					code: m.invalid_union,
					unionErrors: o
				}), w;
			}
		}
		get options() {
			return this._def.options;
		}
	};
	be.create = (e, t) => new be({
		options: e,
		typeName: v.ZodUnion,
		...x(t)
	});
	const F = (e) => e instanceof we ? F(e.schema) : e instanceof V ? F(e.innerType()) : e instanceof ve ? [e.value] : e instanceof Se ? e.options : e instanceof xe ? $.objectValues(e.enum) : e instanceof Te ? F(e._def.innerType) : e instanceof ye ? [void 0] : e instanceof _e ? [null] : e instanceof H ? [void 0, ...F(e.unwrap())] : e instanceof J ? [null, ...F(e.unwrap())] : e instanceof Ft || e instanceof ke ? F(e.unwrap()) : e instanceof $e ? F(e._def.innerType) : [];
	var Td = class Wc extends T {
		_parse(t) {
			const { ctx: a } = this._processInputParams(t);
			if (a.parsedType !== _.object) return y(a, {
				code: m.invalid_type,
				expected: _.object,
				received: a.parsedType
			}), w;
			const r = this.discriminator, s = a.data[r], c = this.optionsMap.get(s);
			return c ? a.common.async ? c._parseAsync({
				data: a.data,
				path: a.path,
				parent: a
			}) : c._parseSync({
				data: a.data,
				path: a.path,
				parent: a
			}) : (y(a, {
				code: m.invalid_union_discriminator,
				options: Array.from(this.optionsMap.keys()),
				path: [r]
			}), w);
		}
		get discriminator() {
			return this._def.discriminator;
		}
		get options() {
			return this._def.options;
		}
		get optionsMap() {
			return this._def.optionsMap;
		}
		static create(t, a, r) {
			const s = /* @__PURE__ */ new Map();
			for (const c of a) {
				const o = F(c.shape[t]);
				if (!o.length) throw new Error(\`A discriminator value for key \\\`\${t}\\\` could not be extracted from all schema options\`);
				for (const d of o) {
					if (s.has(d)) throw new Error(\`Discriminator property \${String(t)} has duplicate value \${String(d)}\`);
					s.set(d, c);
				}
			}
			return new Wc({
				typeName: v.ZodDiscriminatedUnion,
				discriminator: t,
				options: a,
				optionsMap: s,
				...x(r)
			});
		}
	};
	function Ye(e, t) {
		const a = W(e), r = W(t);
		if (e === t) return {
			valid: !0,
			data: e
		};
		if (a === _.object && r === _.object) {
			const s = $.objectKeys(t), c = $.objectKeys(e).filter((d) => s.indexOf(d) !== -1), o = {
				...e,
				...t
			};
			for (const d of c) {
				const p = Ye(e[d], t[d]);
				if (!p.valid) return { valid: !1 };
				o[d] = p.data;
			}
			return {
				valid: !0,
				data: o
			};
		} else if (a === _.array && r === _.array) {
			if (e.length !== t.length) return { valid: !1 };
			const s = [];
			for (let c = 0; c < e.length; c++) {
				const o = e[c], d = t[c], p = Ye(o, d);
				if (!p.valid) return { valid: !1 };
				s.push(p.data);
			}
			return {
				valid: !0,
				data: s
			};
		} else return a === _.date && r === _.date && +e == +t ? {
			valid: !0,
			data: e
		} : { valid: !1 };
	}
	var ge = class extends T {
		_parse(e) {
			const { status: t, ctx: a } = this._processInputParams(e), r = (s, c) => {
				if (Bt(s) || Bt(c)) return w;
				const o = Ye(s.value, c.value);
				return o.valid ? ((jt(s) || jt(c)) && t.dirty(), {
					status: t.value,
					value: o.data
				}) : (y(a, { code: m.invalid_intersection_types }), w);
			};
			return a.common.async ? Promise.all([this._def.left._parseAsync({
				data: a.data,
				path: a.path,
				parent: a
			}), this._def.right._parseAsync({
				data: a.data,
				path: a.path,
				parent: a
			})]).then(([s, c]) => r(s, c)) : r(this._def.left._parseSync({
				data: a.data,
				path: a.path,
				parent: a
			}), this._def.right._parseSync({
				data: a.data,
				path: a.path,
				parent: a
			}));
		}
	};
	ge.create = (e, t, a) => new ge({
		left: e,
		right: t,
		typeName: v.ZodIntersection,
		...x(a)
	});
	var G = class zc extends T {
		_parse(t) {
			const { status: a, ctx: r } = this._processInputParams(t);
			if (r.parsedType !== _.array) return y(r, {
				code: m.invalid_type,
				expected: _.array,
				received: r.parsedType
			}), w;
			if (r.data.length < this._def.items.length) return y(r, {
				code: m.too_small,
				minimum: this._def.items.length,
				inclusive: !0,
				exact: !1,
				type: "array"
			}), w;
			!this._def.rest && r.data.length > this._def.items.length && (y(r, {
				code: m.too_big,
				maximum: this._def.items.length,
				inclusive: !0,
				exact: !1,
				type: "array"
			}), a.dirty());
			const s = [...r.data].map((c, o) => {
				const d = this._def.items[o] || this._def.rest;
				return d ? d._parse(new L(r, c, r.path, o)) : null;
			}).filter((c) => !!c);
			return r.common.async ? Promise.all(s).then((c) => N.mergeArray(a, c)) : N.mergeArray(a, s);
		}
		get items() {
			return this._def.items;
		}
		rest(t) {
			return new zc({
				...this._def,
				rest: t
			});
		}
	};
	G.create = (e, t) => {
		if (!Array.isArray(e)) throw new Error("You must pass an array of schemas to z.tuple([ ... ])");
		return new G({
			items: e,
			typeName: v.ZodTuple,
			rest: null,
			...x(t)
		});
	};
	var $d = class Et extends T {
		get keySchema() {
			return this._def.keyType;
		}
		get valueSchema() {
			return this._def.valueType;
		}
		_parse(t) {
			const { status: a, ctx: r } = this._processInputParams(t);
			if (r.parsedType !== _.object) return y(r, {
				code: m.invalid_type,
				expected: _.object,
				received: r.parsedType
			}), w;
			const s = [], c = this._def.keyType, o = this._def.valueType;
			for (const d in r.data) s.push({
				key: c._parse(new L(r, d, r.path, d)),
				value: o._parse(new L(r, r.data[d], r.path, d)),
				alwaysSet: d in r.data
			});
			return r.common.async ? N.mergeObjectAsync(a, s) : N.mergeObjectSync(a, s);
		}
		get element() {
			return this._def.valueType;
		}
		static create(t, a, r) {
			return a instanceof T ? new Et({
				keyType: t,
				valueType: a,
				typeName: v.ZodRecord,
				...x(r)
			}) : new Et({
				keyType: fe.create(),
				valueType: t,
				typeName: v.ZodRecord,
				...x(a)
			});
		}
	}, Xe = class extends T {
		get keySchema() {
			return this._def.keyType;
		}
		get valueSchema() {
			return this._def.valueType;
		}
		_parse(e) {
			const { status: t, ctx: a } = this._processInputParams(e);
			if (a.parsedType !== _.map) return y(a, {
				code: m.invalid_type,
				expected: _.map,
				received: a.parsedType
			}), w;
			const r = this._def.keyType, s = this._def.valueType, c = [...a.data.entries()].map(([o, d], p) => ({
				key: r._parse(new L(a, o, a.path, [p, "key"])),
				value: s._parse(new L(a, d, a.path, [p, "value"]))
			}));
			if (a.common.async) {
				const o = /* @__PURE__ */ new Map();
				return Promise.resolve().then(async () => {
					for (const d of c) {
						const p = await d.key, f = await d.value;
						if (p.status === "aborted" || f.status === "aborted") return w;
						(p.status === "dirty" || f.status === "dirty") && t.dirty(), o.set(p.value, f.value);
					}
					return {
						status: t.value,
						value: o
					};
				});
			} else {
				const o = /* @__PURE__ */ new Map();
				for (const d of c) {
					const p = d.key, f = d.value;
					if (p.status === "aborted" || f.status === "aborted") return w;
					(p.status === "dirty" || f.status === "dirty") && t.dirty(), o.set(p.value, f.value);
				}
				return {
					status: t.value,
					value: o
				};
			}
		}
	};
	Xe.create = (e, t, a) => new Xe({
		valueType: t,
		keyType: e,
		typeName: v.ZodMap,
		...x(a)
	});
	var Qe = class At extends T {
		_parse(t) {
			const { status: a, ctx: r } = this._processInputParams(t);
			if (r.parsedType !== _.set) return y(r, {
				code: m.invalid_type,
				expected: _.set,
				received: r.parsedType
			}), w;
			const s = this._def;
			s.minSize !== null && r.data.size < s.minSize.value && (y(r, {
				code: m.too_small,
				minimum: s.minSize.value,
				type: "set",
				inclusive: !0,
				exact: !1,
				message: s.minSize.message
			}), a.dirty()), s.maxSize !== null && r.data.size > s.maxSize.value && (y(r, {
				code: m.too_big,
				maximum: s.maxSize.value,
				type: "set",
				inclusive: !0,
				exact: !1,
				message: s.maxSize.message
			}), a.dirty());
			const c = this._def.valueType;
			function o(p) {
				const f = /* @__PURE__ */ new Set();
				for (const R of p) {
					if (R.status === "aborted") return w;
					R.status === "dirty" && a.dirty(), f.add(R.value);
				}
				return {
					status: a.value,
					value: f
				};
			}
			const d = [...r.data.values()].map((p, f) => c._parse(new L(r, p, r.path, f)));
			return r.common.async ? Promise.all(d).then((p) => o(p)) : o(d);
		}
		min(t, a) {
			return new At({
				...this._def,
				minSize: {
					value: t,
					message: b.toString(a)
				}
			});
		}
		max(t, a) {
			return new At({
				...this._def,
				maxSize: {
					value: t,
					message: b.toString(a)
				}
			});
		}
		size(t, a) {
			return this.min(t, a).max(t, a);
		}
		nonempty(t) {
			return this.min(1, t);
		}
	};
	Qe.create = (e, t) => new Qe({
		valueType: e,
		minSize: null,
		maxSize: null,
		typeName: v.ZodSet,
		...x(t)
	});
	var kd = class Ze extends T {
		constructor() {
			super(...arguments), this.validate = this.implement;
		}
		_parse(t) {
			const { ctx: a } = this._processInputParams(t);
			if (a.parsedType !== _.function) return y(a, {
				code: m.invalid_type,
				expected: _.function,
				received: a.parsedType
			}), w;
			function r(d, p) {
				return Ue({
					data: d,
					path: a.path,
					errorMaps: [
						a.common.contextualErrorMap,
						a.schemaErrorMap,
						qe(),
						ie
					].filter((f) => !!f),
					issueData: {
						code: m.invalid_arguments,
						argumentsError: p
					}
				});
			}
			function s(d, p) {
				return Ue({
					data: d,
					path: a.path,
					errorMaps: [
						a.common.contextualErrorMap,
						a.schemaErrorMap,
						qe(),
						ie
					].filter((f) => !!f),
					issueData: {
						code: m.invalid_return_type,
						returnTypeError: p
					}
				});
			}
			const c = { errorMap: a.common.contextualErrorMap }, o = a.data;
			if (this._def.returns instanceof oe) {
				const d = this;
				return E(async function(...p) {
					const f = new j([]), R = await d._def.args.parseAsync(p, c).catch((D) => {
						throw f.addIssue(r(p, D)), f;
					}), P = await Reflect.apply(o, this, R);
					return await d._def.returns._def.type.parseAsync(P, c).catch((D) => {
						throw f.addIssue(s(P, D)), f;
					});
				});
			} else {
				const d = this;
				return E(function(...p) {
					const f = d._def.args.safeParse(p, c);
					if (!f.success) throw new j([r(p, f.error)]);
					const R = Reflect.apply(o, this, f.data), P = d._def.returns.safeParse(R, c);
					if (!P.success) throw new j([s(R, P.error)]);
					return P.data;
				});
			}
		}
		parameters() {
			return this._def.args;
		}
		returnType() {
			return this._def.returns;
		}
		args(...t) {
			return new Ze({
				...this._def,
				args: G.create(t).rest(X.create())
			});
		}
		returns(t) {
			return new Ze({
				...this._def,
				returns: t
			});
		}
		implement(t) {
			return this.parse(t);
		}
		strictImplement(t) {
			return this.parse(t);
		}
		static create(t, a, r) {
			return new Ze({
				args: t || G.create([]).rest(X.create()),
				returns: a || X.create(),
				typeName: v.ZodFunction,
				...x(r)
			});
		}
	}, we = class extends T {
		get schema() {
			return this._def.getter();
		}
		_parse(e) {
			const { ctx: t } = this._processInputParams(e);
			return this._def.getter()._parse({
				data: t.data,
				path: t.path,
				parent: t
			});
		}
	};
	we.create = (e, t) => new we({
		getter: e,
		typeName: v.ZodLazy,
		...x(t)
	});
	var ve = class extends T {
		_parse(e) {
			if (e.data !== this._def.value) {
				const t = this._getOrReturnCtx(e);
				return y(t, {
					received: t.data,
					code: m.invalid_literal,
					expected: this._def.value
				}), w;
			}
			return {
				status: "valid",
				value: e.data
			};
		}
		get value() {
			return this._def.value;
		}
	};
	ve.create = (e, t) => new ve({
		value: e,
		typeName: v.ZodLiteral,
		...x(t)
	});
	function Ut(e, t) {
		return new Se({
			values: e,
			typeName: v.ZodEnum,
			...x(t)
		});
	}
	var Se = class Nt extends T {
		_parse(t) {
			if (typeof t.data != "string") {
				const a = this._getOrReturnCtx(t), r = this._def.values;
				return y(a, {
					expected: $.joinValues(r),
					received: a.parsedType,
					code: m.invalid_type
				}), w;
			}
			if (this._cache || (this._cache = new Set(this._def.values)), !this._cache.has(t.data)) {
				const a = this._getOrReturnCtx(t), r = this._def.values;
				return y(a, {
					received: a.data,
					code: m.invalid_enum_value,
					options: r
				}), w;
			}
			return E(t.data);
		}
		get options() {
			return this._def.values;
		}
		get enum() {
			const t = {};
			for (const a of this._def.values) t[a] = a;
			return t;
		}
		get Values() {
			const t = {};
			for (const a of this._def.values) t[a] = a;
			return t;
		}
		get Enum() {
			const t = {};
			for (const a of this._def.values) t[a] = a;
			return t;
		}
		extract(t, a = this._def) {
			return Nt.create(t, {
				...this._def,
				...a
			});
		}
		exclude(t, a = this._def) {
			return Nt.create(this.options.filter((r) => !t.includes(r)), {
				...this._def,
				...a
			});
		}
	};
	Se.create = Ut;
	var xe = class extends T {
		_parse(e) {
			const t = $.getValidEnumValues(this._def.values), a = this._getOrReturnCtx(e);
			if (a.parsedType !== _.string && a.parsedType !== _.number) {
				const r = $.objectValues(t);
				return y(a, {
					expected: $.joinValues(r),
					received: a.parsedType,
					code: m.invalid_type
				}), w;
			}
			if (this._cache || (this._cache = new Set($.getValidEnumValues(this._def.values))), !this._cache.has(e.data)) {
				const r = $.objectValues(t);
				return y(a, {
					received: a.data,
					code: m.invalid_enum_value,
					options: r
				}), w;
			}
			return E(e.data);
		}
		get enum() {
			return this._def.values;
		}
	};
	xe.create = (e, t) => new xe({
		values: e,
		typeName: v.ZodNativeEnum,
		...x(t)
	});
	var oe = class extends T {
		unwrap() {
			return this._def.type;
		}
		_parse(e) {
			const { ctx: t } = this._processInputParams(e);
			return t.parsedType !== _.promise && t.common.async === !1 ? (y(t, {
				code: m.invalid_type,
				expected: _.promise,
				received: t.parsedType
			}), w) : E((t.parsedType === _.promise ? t.data : Promise.resolve(t.data)).then((a) => this._def.type.parseAsync(a, {
				path: t.path,
				errorMap: t.common.contextualErrorMap
			})));
		}
	};
	oe.create = (e, t) => new oe({
		type: e,
		typeName: v.ZodPromise,
		...x(t)
	});
	var V = class extends T {
		innerType() {
			return this._def.schema;
		}
		sourceType() {
			return this._def.schema._def.typeName === v.ZodEffects ? this._def.schema.sourceType() : this._def.schema;
		}
		_parse(e) {
			const { status: t, ctx: a } = this._processInputParams(e), r = this._def.effect || null, s = {
				addIssue: (c) => {
					y(a, c), c.fatal ? t.abort() : t.dirty();
				},
				get path() {
					return a.path;
				}
			};
			if (s.addIssue = s.addIssue.bind(s), r.type === "preprocess") {
				const c = r.transform(a.data, s);
				if (a.common.async) return Promise.resolve(c).then(async (o) => {
					if (t.value === "aborted") return w;
					const d = await this._def.schema._parseAsync({
						data: o,
						path: a.path,
						parent: a
					});
					return d.status === "aborted" ? w : d.status === "dirty" || t.value === "dirty" ? ne(d.value) : d;
				});
				{
					if (t.value === "aborted") return w;
					const o = this._def.schema._parseSync({
						data: c,
						path: a.path,
						parent: a
					});
					return o.status === "aborted" ? w : o.status === "dirty" || t.value === "dirty" ? ne(o.value) : o;
				}
			}
			if (r.type === "refinement") {
				const c = (o) => {
					const d = r.refinement(o, s);
					if (a.common.async) return Promise.resolve(d);
					if (d instanceof Promise) throw new Error("Async refinement encountered during synchronous parse operation. Use .parseAsync instead.");
					return o;
				};
				if (a.common.async === !1) {
					const o = this._def.schema._parseSync({
						data: a.data,
						path: a.path,
						parent: a
					});
					return o.status === "aborted" ? w : (o.status === "dirty" && t.dirty(), c(o.value), {
						status: t.value,
						value: o.value
					});
				} else return this._def.schema._parseAsync({
					data: a.data,
					path: a.path,
					parent: a
				}).then((o) => o.status === "aborted" ? w : (o.status === "dirty" && t.dirty(), c(o.value).then(() => ({
					status: t.value,
					value: o.value
				}))));
			}
			if (r.type === "transform") if (a.common.async === !1) {
				const c = this._def.schema._parseSync({
					data: a.data,
					path: a.path,
					parent: a
				});
				if (!Y(c)) return w;
				const o = r.transform(c.value, s);
				if (o instanceof Promise) throw new Error("Asynchronous transform encountered during synchronous parse operation. Use .parseAsync instead.");
				return {
					status: t.value,
					value: o
				};
			} else return this._def.schema._parseAsync({
				data: a.data,
				path: a.path,
				parent: a
			}).then((c) => Y(c) ? Promise.resolve(r.transform(c.value, s)).then((o) => ({
				status: t.value,
				value: o
			})) : w);
			$.assertNever(r);
		}
	};
	V.create = (e, t, a) => new V({
		schema: e,
		typeName: v.ZodEffects,
		effect: t,
		...x(a)
	}), V.createWithPreprocess = (e, t, a) => new V({
		schema: t,
		effect: {
			type: "preprocess",
			transform: e
		},
		typeName: v.ZodEffects,
		...x(a)
	});
	var H = class extends T {
		_parse(e) {
			return this._getType(e) === _.undefined ? E(void 0) : this._def.innerType._parse(e);
		}
		unwrap() {
			return this._def.innerType;
		}
	};
	H.create = (e, t) => new H({
		innerType: e,
		typeName: v.ZodOptional,
		...x(t)
	});
	var J = class extends T {
		_parse(e) {
			return this._getType(e) === _.null ? E(null) : this._def.innerType._parse(e);
		}
		unwrap() {
			return this._def.innerType;
		}
	};
	J.create = (e, t) => new J({
		innerType: e,
		typeName: v.ZodNullable,
		...x(t)
	});
	var Te = class extends T {
		_parse(e) {
			const { ctx: t } = this._processInputParams(e);
			let a = t.data;
			return t.parsedType === _.undefined && (a = this._def.defaultValue()), this._def.innerType._parse({
				data: a,
				path: t.path,
				parent: t
			});
		}
		removeDefault() {
			return this._def.innerType;
		}
	};
	Te.create = (e, t) => new Te({
		innerType: e,
		typeName: v.ZodDefault,
		defaultValue: typeof t.default == "function" ? t.default : () => t.default,
		...x(t)
	});
	var $e = class extends T {
		_parse(e) {
			const { ctx: t } = this._processInputParams(e), a = {
				...t,
				common: {
					...t.common,
					issues: []
				}
			}, r = this._def.innerType._parse({
				data: a.data,
				path: a.path,
				parent: { ...a }
			});
			return pe(r) ? r.then((s) => ({
				status: "valid",
				value: s.status === "valid" ? s.value : this._def.catchValue({
					get error() {
						return new j(a.common.issues);
					},
					input: a.data
				})
			})) : {
				status: "valid",
				value: r.status === "valid" ? r.value : this._def.catchValue({
					get error() {
						return new j(a.common.issues);
					},
					input: a.data
				})
			};
		}
		removeCatch() {
			return this._def.innerType;
		}
	};
	$e.create = (e, t) => new $e({
		innerType: e,
		typeName: v.ZodCatch,
		catchValue: typeof t.catch == "function" ? t.catch : () => t.catch,
		...x(t)
	});
	var et = class extends T {
		_parse(e) {
			if (this._getType(e) !== _.nan) {
				const t = this._getOrReturnCtx(e);
				return y(t, {
					code: m.invalid_type,
					expected: _.nan,
					received: t.parsedType
				}), w;
			}
			return {
				status: "valid",
				value: e.data
			};
		}
	};
	et.create = (e) => new et({
		typeName: v.ZodNaN,
		...x(e)
	});
	var Ft = class extends T {
		_parse(e) {
			const { ctx: t } = this._processInputParams(e), a = t.data;
			return this._def.type._parse({
				data: a,
				path: t.path,
				parent: t
			});
		}
		unwrap() {
			return this._def.type;
		}
	}, Vt = class Gc extends T {
		_parse(t) {
			const { status: a, ctx: r } = this._processInputParams(t);
			if (r.common.async) return (async () => {
				const c = await this._def.in._parseAsync({
					data: r.data,
					path: r.path,
					parent: r
				});
				return c.status === "aborted" ? w : c.status === "dirty" ? (a.dirty(), ne(c.value)) : this._def.out._parseAsync({
					data: c.value,
					path: r.path,
					parent: r
				});
			})();
			{
				const s = this._def.in._parseSync({
					data: r.data,
					path: r.path,
					parent: r
				});
				return s.status === "aborted" ? w : s.status === "dirty" ? (a.dirty(), {
					status: "dirty",
					value: s.value
				}) : this._def.out._parseSync({
					data: s.value,
					path: r.path,
					parent: r
				});
			}
		}
		static create(t, a) {
			return new Gc({
				in: t,
				out: a,
				typeName: v.ZodPipeline
			});
		}
	}, ke = class extends T {
		_parse(e) {
			const t = this._def.innerType._parse(e), a = (r) => (Y(r) && (r.value = Object.freeze(r.value)), r);
			return pe(t) ? t.then((r) => a(r)) : a(t);
		}
		unwrap() {
			return this._def.innerType;
		}
	};
	ke.create = (e, t) => new ke({
		innerType: e,
		typeName: v.ZodReadonly,
		...x(t)
	});
	O.lazycreate;
	var v;
	(function(e) {
		e.ZodString = "ZodString", e.ZodNumber = "ZodNumber", e.ZodNaN = "ZodNaN", e.ZodBigInt = "ZodBigInt", e.ZodBoolean = "ZodBoolean", e.ZodDate = "ZodDate", e.ZodSymbol = "ZodSymbol", e.ZodUndefined = "ZodUndefined", e.ZodNull = "ZodNull", e.ZodAny = "ZodAny", e.ZodUnknown = "ZodUnknown", e.ZodNever = "ZodNever", e.ZodVoid = "ZodVoid", e.ZodArray = "ZodArray", e.ZodObject = "ZodObject", e.ZodUnion = "ZodUnion", e.ZodDiscriminatedUnion = "ZodDiscriminatedUnion", e.ZodIntersection = "ZodIntersection", e.ZodTuple = "ZodTuple", e.ZodRecord = "ZodRecord", e.ZodMap = "ZodMap", e.ZodSet = "ZodSet", e.ZodFunction = "ZodFunction", e.ZodLazy = "ZodLazy", e.ZodLiteral = "ZodLiteral", e.ZodEnum = "ZodEnum", e.ZodEffects = "ZodEffects", e.ZodNativeEnum = "ZodNativeEnum", e.ZodOptional = "ZodOptional", e.ZodNullable = "ZodNullable", e.ZodDefault = "ZodDefault", e.ZodCatch = "ZodCatch", e.ZodPromise = "ZodPromise", e.ZodBranded = "ZodBranded", e.ZodPipeline = "ZodPipeline", e.ZodReadonly = "ZodReadonly";
	})(v || (v = {}));
	const i = fe.create, u = Ve.create;
	et.create;
	He.create;
	const k = We.create;
	ze.create;
	Ge.create;
	ye.create;
	_e.create;
	Je.create;
	const Z = X.create;
	z.create;
	Ke.create;
	const g = Q.create, n = O.create;
	O.strictCreate;
	const Ht = be.create, ce = Td.create;
	ge.create;
	const tt = G.create, de = $d.create;
	Xe.create;
	Qe.create;
	kd.create;
	we.create;
	const C = ve.create, S = Se.create;
	xe.create;
	oe.create;
	V.create;
	H.create;
	J.create;
	V.createWithPreprocess;
	Vt.create;
	const Wt = n({
		id: i(),
		description: i()
	}), Rd = n({
		browser: g(Wt).optional(),
		tab: g(Wt).optional()
	}), Cd = S([
		"iab",
		"extension",
		"cdp",
		"mcpapps"
	]), zt = n({
		family: i().trim().min(1).optional(),
		id: i(),
		name: i(),
		type: Cd,
		profileName: i().optional(),
		metadata: n({
			extensionInstanceId: i().optional(),
			codexSessionId: i().optional()
		}).optional()
	}), at = zt.extend({
		apiSupportOverrides: de(k()).optional(),
		capabilities: Rd
	});
	var Pd = l({
		PayloadSchema: () => Gt,
		ResultSchema: () => Ed,
		commandType: () => Jt,
		create: () => Kt
	});
	const Gt = n({}), Ed = g(zt), Jt = "list_browsers";
	function Kt(e = {}) {
		return new h(Jt, Gt, e);
	}
	var Ad = l({
		PayloadSchema: () => Yt,
		ResultSchema: () => Nd,
		commandType: () => Xt,
		create: () => Qt
	});
	const Yt = n({ id: i() }), Nd = at, Xt = "get_browser";
	function Qt(e) {
		return new h(Xt, Yt, e);
	}
	var Dd = l({
		PayloadSchema: () => ea,
		ResultSchema: () => Od,
		commandType: () => ta,
		create: () => aa
	});
	const ea = n({}), Od = at, ta = "get_default_browser";
	function aa(e = {}) {
		return new h(ta, ea, e);
	}
	var Md = l({
		PayloadSchema: () => ra,
		ResultSchema: () => Bd,
		commandType: () => sa,
		create: () => ia
	});
	const ra = n({ name: i() }), Bd = i(), sa = "get_documentation";
	function ia(e) {
		return new h(sa, ra, e);
	}
	var jd = l({
		PayloadSchema: () => na,
		ResultSchema: () => Ld,
		commandType: () => oa,
		create: () => ca
	});
	const na = n({ browser_id: i() }), Ld = i(), oa = "get_browser_documentation";
	function ca(e) {
		return new h(oa, na, e);
	}
	var Zd = l({
		PayloadSchema: () => da,
		ResultSchema: () => qd,
		commandType: () => la,
		create: () => ua
	});
	const da = n({ url: i().url() }), qd = at, la = "get_browser_for_url";
	function ua(e) {
		return new h(la, da, e);
	}
	var Ud = l({
		PayloadSchema: () => ha,
		ResultSchema: () => Fd,
		commandType: () => ma,
		create: () => pa
	});
	const ha = n({ browser_id: i() }), Fd = n({ id: i() }), ma = "create_tab";
	function pa(e) {
		return new h(ma, ha, e);
	}
	var Vd = l({
		PayloadSchema: () => fa,
		ResultSchema: () => Hd,
		commandType: () => ya,
		create: () => rt
	});
	const fa = n({
		browser_id: i(),
		tab_id: i()
	}), Hd = n({
		id: i(),
		title: i().optional(),
		url: i().optional()
	}), ya = "get_tab";
	function rt(e) {
		return new h(ya, fa, e);
	}
	var Wd = l({
		PayloadSchema: () => _a,
		ResultSchema: () => zd,
		commandType: () => ba,
		create: () => ga
	});
	const _a = n({
		browser_id: i(),
		tab_id: i()
	}), zd = n({}), ba = "close_tab";
	function ga(e) {
		return new h(ba, _a, e);
	}
	var Gd = l({
		MarkTabStatusSchema: () => wa,
		PayloadSchema: () => va,
		ResultSchema: () => Jd,
		commandType: () => Sa,
		create: () => xa
	});
	const wa = S(["handoff", "deliverable"]), va = n({
		browser_id: i(),
		tab_id: i(),
		status: wa
	}), Jd = n({}), Sa = "mark_tab";
	function xa(e) {
		return new h(Sa, va, e);
	}
	var Kd = l({
		PayloadSchema: () => ka,
		ReasonSchema: () => $a,
		ResultSchema: () => Yd,
		commandType: () => Ta,
		create: () => Ra
	});
	const Ta = "tab_manual_handoff_request", $a = S([
		"authentication",
		"captcha",
		"user_requested",
		"other"
	]), ka = n({
		browser_id: i(),
		tab_id: i(),
		reason: $a.optional()
	}), Yd = n({});
	function Ra(e) {
		return new h(Ta, ka, e);
	}
	const st = ce("kind", [n({
		kind: C("ax"),
		snapshotId: i().min(1),
		elementIndex: u().int().nonnegative()
	}), n({
		kind: C("dom"),
		snapshotId: i().min(1),
		ref: i().min(1)
	})]), le = n({
		evidence: S(["siwc_marker_detected", "chatgpt_button_detected"]),
		label: i().optional(),
		visibility: S(["rendered", "hidden"]),
		target: st
	}), Re = ce("status", [
		n({
			status: C("possible_matches"),
			candidates: g(le).nonempty()
		}),
		n({
			status: C("no_deterministic_matches"),
			candidates: tt([])
		}),
		n({
			status: C("unavailable"),
			reason: S(["page_changed", "observation_unavailable"])
		})
	]), Ca = n({ target: st.optional() });
	var Xd = l({
		PayloadSchema: () => Pa,
		ResultSchema: () => Qd,
		commandType: () => Ia,
		create: () => Ea
	});
	const Ia = "tab_siwc_authenticate", Pa = Ca.extend({
		browser_id: i(),
		tab_id: i()
	}), Qd = ce("status", [n({ status: S([
		"initiated",
		"unsupported",
		"no_actionable_button",
		"target_unavailable",
		"page_changed",
		"account_unavailable",
		"account_changed",
		"approval_declined",
		"approval_cancelled",
		"approval_failed",
		"grant_failed",
		"cookie_failed",
		"click_failed"
	]) }), n({
		status: C("multiple_actionable_buttons"),
		candidates: tt([le, le]).rest(le)
	})]);
	function Ea(e) {
		return new h(Ia, Pa, e);
	}
	var el = l({
		PayloadSchema: () => Aa,
		ResultSchema: () => tl,
		commandType: () => Na,
		create: () => Da
	});
	const Aa = n({
		browser_id: i(),
		tab_id: i()
	}), tl = Re, Na = "tab_siwc_detect";
	function Da(e) {
		return new h(Na, Aa, e);
	}
	var al = l({
		PayloadSchema: () => Oa,
		ResultSchema: () => rl,
		commandType: () => Ma,
		create: () => sl
	});
	const Oa = n({ browser_id: i() }), rl = n({ display_truncate_max_chars: u().int().positive().optional() }), Ma = "runtime_config";
	function sl(e) {
		return new h(Ma, Oa, e);
	}
	var il = l({
		PayloadSchema: () => Ba,
		ResultSchema: () => nl,
		commandType: () => ja,
		create: () => La
	});
	const Ba = n({
		browser_id: i(),
		name: i()
	}), nl = n({}), ja = "name_session";
	function La(e) {
		return new h(ja, Ba, e);
	}
	var ol = l({
		PayloadSchema: () => Za,
		ResultSchema: () => cl,
		commandType: () => qa,
		create: () => Ua
	});
	const Za = n({ browser_id: i() }), cl = n({ id: i().optional() }), qa = "selected_tab";
	function Ua(e) {
		return new h(qa, Za, e);
	}
	var dl = l({
		PayloadSchema: () => Fa,
		ResultSchema: () => ll,
		commandType: () => Va,
		create: () => Ha
	});
	const Fa = n({ browser_id: i() }), ll = n({ tabs: g(n({
		id: i(),
		providerTabId: i().optional(),
		url: i().optional(),
		title: i().optional()
	})) }), Va = "list_tabs";
	function Ha(e) {
		return new h(Va, Fa, e);
	}
	var ul = l({
		ContentTypeSchema: () => Wa,
		PayloadSchema: () => za,
		ResultSchema: () => hl,
		commandType: () => Ga,
		create: () => Ja
	});
	const Wa = S([
		"html",
		"text",
		"domSnapshot"
	]), za = n({
		browser_id: i(),
		urls: g(i()),
		content_type: Wa,
		timeout_ms: u().int().positive().optional()
	}), hl = n({ results: g(n({
		url: i(),
		title: i().nullable(),
		content: i().nullable()
	})) }), Ga = "tabs_content";
	function Ja(e) {
		return new h(Ga, za, e);
	}
	const ml = n({
		readOnlyHint: k().optional(),
		untrustedContentHint: k().optional(),
		consequentialHint: k().optional()
	}), pl = n({
		name: i(),
		call_name: i(),
		registration_id: i(),
		title: i().optional(),
		description: i().optional(),
		input_schema: Z(),
		annotations: ml.optional(),
		origin: i().optional(),
		pageUrl: i().optional()
	});
	var fl = l({
		PayloadSchema: () => Ka,
		ResultSchema: () => Ya,
		commandType: () => Xa,
		create: () => Qa
	});
	const Ka = n({
		browser_id: i(),
		tab_id: i()
	}), Ya = n({ tools: g(pl) }), Xa = "webmcp_list_tools";
	function Qa(e) {
		return new h(Xa, Ka, e);
	}
	var yl = l({
		PayloadSchema: () => er,
		ResultSchema: () => _l,
		commandType: () => tr,
		create: () => ar
	});
	const er = n({
		browser_id: i(),
		tab_id: i(),
		tool_name: i(),
		tool_description: i().optional(),
		tool_origin: i().optional(),
		tool_title: i().optional(),
		registration_id: i(),
		input: Z(),
		timeout_ms: u().int().positive().optional()
	}), _l = n({ result: Z() }), tr = "webmcp_invoke_tool";
	function ar(e) {
		return new h(tr, er, e);
	}
	var bl = l({
		PayloadSchema: () => rr,
		ResultSchema: () => gl,
		commandType: () => sr,
		create: () => ir
	});
	const rr = n({
		browser_id: i(),
		tab_id: i(),
		url: i()
	}), gl = n({}), sr = "navigate_tab_url";
	function ir(e) {
		return new h(sr, rr, e);
	}
	var wl = l({
		PayloadSchema: () => nr,
		ResultSchema: () => vl,
		commandType: () => or,
		create: () => it
	});
	const nr = n({
		browser_id: i(),
		tab_id: i()
	}), vl = n({}), or = "navigate_tab_back";
	function it(e) {
		return new h(or, nr, e);
	}
	var Sl = l({
		PayloadSchema: () => cr,
		ResultSchema: () => xl,
		commandType: () => dr,
		create: () => nt
	});
	const cr = n({
		browser_id: i(),
		tab_id: i()
	}), xl = n({}), dr = "navigate_tab_forward";
	function nt(e) {
		return new h(dr, cr, e);
	}
	var Tl = l({
		PayloadSchema: () => lr,
		ResultSchema: () => $l,
		commandType: () => ur,
		create: () => hr
	});
	const lr = n({
		browser_id: i(),
		tab_id: i()
	}), $l = n({}), ur = "navigate_tab_reload";
	function hr(e) {
		return new h(ur, lr, e);
	}
	var kl = l({
		PayloadSchema: () => mr,
		ResultSchema: () => Rl,
		commandType: () => pr,
		create: () => fr
	});
	const mr = n({
		browser_id: i(),
		tab_id: i()
	}), Rl = n({
		id: i(),
		title: i().optional(),
		url: i().optional()
	}), pr = "browser_user_claim_tab";
	function fr(e) {
		return new h(pr, mr, e);
	}
	var Cl = l({
		PayloadSchema: () => yr,
		ResultSchema: () => _r,
		commandType: () => br,
		create: () => gr
	});
	const yr = n({
		browser_id: i(),
		tab_id: i().optional()
	}), _r = ce("kind", [
		n({
			kind: C("library_file"),
			libraryFileId: i().min(1),
			fileId: i().min(1),
			fileName: i(),
			mimeType: i(),
			title: i(),
			url: i()
		}),
		n({
			kind: C("text"),
			text: i(),
			title: i(),
			truncated: k(),
			url: i()
		}),
		n({
			dataBase64: i(),
			fileName: i(),
			kind: C("document"),
			mimeType: i(),
			title: i(),
			url: i()
		})
	]), br = "browser_user_get_tab_context";
	function gr(e) {
		return new h(br, yr, e);
	}
	var Il = l({
		PayloadSchema: () => wr,
		ResultSchema: () => Pl,
		commandType: () => vr,
		create: () => Sr
	});
	const wr = n({ browser_id: i() }), Pl = n({ tabs: g(n({
		id: i(),
		providerTabId: i().optional(),
		title: i().optional(),
		url: i().optional(),
		lastOpened: i().optional(),
		tabGroup: i().optional()
	})) }), vr = "browser_user_open_tabs";
	function Sr(e) {
		return new h(vr, wr, e);
	}
	var El = l({
		PayloadSchema: () => xr,
		ResultSchema: () => Al,
		commandType: () => Tr,
		create: () => $r
	});
	const xr = n({
		browser_id: i(),
		queries: g(i()).min(1).optional(),
		limit: u().int().positive().optional(),
		from: i().optional(),
		to: i().optional()
	}), Al = n({ items: g(n({
		url: i(),
		title: i().optional(),
		dateVisited: i()
	})) }), Tr = "browser_user_history";
	function $r(e) {
		return new h(Tr, xr, e);
	}
	var Nl = l({
		LogLevelSchema: () => Ce,
		PayloadSchema: () => kr,
		ResultSchema: () => Dl,
		commandType: () => Rr,
		create: () => Cr
	});
	const Ce = S([
		"debug",
		"info",
		"log",
		"warn",
		"error"
	]), kr = n({
		browser_id: i(),
		tab_id: i(),
		filter: i().optional(),
		levels: g(Ce).optional(),
		limit: u().int().positive().optional()
	}), Dl = n({ logs: g(n({
		level: Ce,
		message: i(),
		timestamp: i(),
		url: i().optional()
	})) }), Rr = "tab_dev_logs";
	function Cr(e) {
		return new h(Rr, kr, e);
	}
	var Ol = l({
		JsDialogTypeSchema: () => Ir,
		PayloadSchema: () => Er,
		ResultDialogSchema: () => Pr,
		ResultSchema: () => Ml,
		commandType: () => Ar,
		create: () => Nr
	});
	const Ir = S([
		"alert",
		"beforeunload",
		"confirm",
		"prompt"
	]), Pr = n({
		id: i(),
		type: Ir
	}), Er = n({
		browser_id: i(),
		tab_id: i()
	}), Ml = n({ dialog: Pr.nullable() }), Ar = "tab_get_js_dialog";
	function Nr(e) {
		return new h(Ar, Er, e);
	}
	var Bl = l({
		PayloadSchema: () => Dr,
		ResultSchema: () => jl,
		commandType: () => Or,
		create: () => Ie
	});
	const Dr = n({
		action: S(["accept", "dismiss"]),
		browser_id: i(),
		dialog_id: i(),
		prompt_text: i().optional(),
		tab_id: i()
	}), jl = n({}), Or = "tab_handle_js_dialog";
	function Ie(e) {
		return new h(Or, Dr, e);
	}
	var Ll = l({
		PayloadSchema: () => Mr,
		ResultSchema: () => Zl,
		commandType: () => Br,
		create: () => jr
	});
	const Mr = n({
		browser_id: i(),
		tab_id: i()
	}), Zl = n({ path: i() }), Br = "tab_content_export";
	function jr(e) {
		return new h(Br, Mr, e);
	}
	var ql = l({
		GSuiteExportTypeSchema: () => Lr,
		PayloadSchema: () => Zr,
		ResultSchema: () => Ul,
		commandType: () => qr,
		create: () => Ur
	});
	const Lr = S([
		"pdf",
		"md",
		"xlsx",
		"csv",
		"docx",
		"pptx"
	]), Zr = n({
		browser_id: i(),
		tab_id: i(),
		format: Lr
	}), Ul = n({ path: i() }), qr = "tab_content_export_gsuite";
	function Ur(e) {
		return new h(qr, Zr, e);
	}
	var Fl = l({
		PayloadSchema: () => Fr,
		ResultSchema: () => Vl,
		commandType: () => Vr,
		create: () => Hr
	});
	const Fr = n({
		browser_id: i(),
		tab_id: i()
	}), Vl = n({ path: i() }), Vr = "tab_content_export_youtube_transcript";
	function Hr(e) {
		return new h(Vr, Fr, e);
	}
	var Hl = l({
		PayloadSchema: () => Wr,
		ResultSchema: () => Wl,
		commandType: () => zr,
		create: () => Gr
	});
	const Wr = n({
		browser_id: i(),
		tab_id: i()
	}), Wl = n({ text: i() }), zr = "tab_clipboard_read_text";
	function Gr(e) {
		return new h(zr, Wr, e);
	}
	var zl = l({
		PayloadSchema: () => Jr,
		ResultSchema: () => Gl,
		commandType: () => Kr,
		create: () => Yr
	});
	const Jr = n({
		browser_id: i(),
		tab_id: i(),
		text: i()
	}), Gl = n({}), Kr = "tab_clipboard_write_text";
	function Yr(e) {
		return new h(Kr, Jr, e);
	}
	var Jl = l({
		PayloadSchema: () => Xr,
		ResultSchema: () => Xl,
		commandType: () => Qr,
		create: () => es
	});
	const Yl = n({
		entries: g(n({
			mime_type: i(),
			text: i().optional(),
			base64: i().optional()
		}).superRefine((e, t) => {
			e.text !== void 0 == (e.base64 !== void 0) && t.addIssue({
				code: m.custom,
				message: "Clipboard entries must set exactly one of text or base64"
			});
		})),
		presentation_style: S([
			"unspecified",
			"inline",
			"attachment"
		]).optional()
	}), Xr = n({
		browser_id: i(),
		tab_id: i()
	}), Xl = n({ items: g(Yl) }), Qr = "tab_clipboard_read";
	function es(e) {
		return new h(Qr, Xr, e);
	}
	var Ql = l({
		PayloadSchema: () => ts,
		ResultSchema: () => au,
		commandType: () => as,
		create: () => rs
	});
	const tu = n({
		entries: g(n({
			mime_type: i(),
			text: i().optional(),
			base64: i().optional()
		}).superRefine((e, t) => {
			e.text !== void 0 == (e.base64 !== void 0) && t.addIssue({
				code: m.custom,
				message: "Clipboard entries must set exactly one of text or base64"
			});
		})),
		presentation_style: S([
			"unspecified",
			"inline",
			"attachment"
		]).optional()
	}), ts = n({
		browser_id: i(),
		tab_id: i(),
		items: g(tu)
	}), au = n({}), as = "tab_clipboard_write";
	function rs(e) {
		return new h(as, ts, e);
	}
	var ru = l({
		PayloadSchema: () => ss,
		ResultSchema: () => su,
		commandType: () => is,
		create: () => ns
	});
	const ss = n({
		browser_id: i(),
		tab_id: i(),
		fullPage: k().optional(),
		cropX: u().optional(),
		cropY: u().optional(),
		cropWidth: u().optional(),
		cropHeight: u().optional()
	}), su = n({ data: i() }), is = "tab_screenshot";
	function ns(e) {
		return new h(is, ss, e);
	}
	var iu = l({
		PayloadSchema: () => os,
		ResultSchema: () => nu,
		commandType: () => cs,
		create: () => ds
	});
	const os = n({
		browser_id: i(),
		tab_id: i(),
		content: S([
			"axState",
			"screenshot",
			"axStateAndScreenshot"
		]),
		disable_diffing: k().optional()
	}), nu = n({
		state: i().optional(),
		data: i().optional(),
		screenshot_unavailable: i().optional(),
		snapshotId: i().optional(),
		siwc: Re.optional()
	}), cs = "tab_ax_get_state";
	function ds(e) {
		return new h(cs, os, e);
	}
	var ou = l({
		ActionSchema: () => us,
		PayloadSchema: () => hs,
		ResultSchema: () => cu,
		commandType: () => ms,
		create: () => ps
	});
	const ot = tt([u(), u()]), K = u().int().nonnegative(), ls = Ht([K, ot]), us = ce("kind", [
		n({
			kind: C("paste"),
			element_index: K.nullable(),
			text: i(),
			format: S([
				"text",
				"md",
				"html"
			]).optional()
		}),
		n({
			kind: C("click"),
			target: ls,
			mouse_button: S([
				"left",
				"right",
				"middle",
				"l",
				"r",
				"m"
			]).optional(),
			click_count: u().int().positive().optional(),
			key: i().optional(),
			duration: u().finite().min(0).max(30).optional()
		}),
		n({
			kind: C("drag"),
			from: ot,
			to: ot
		}),
		n({
			kind: C("perform_secondary_action"),
			element_index: K,
			action: i()
		}),
		n({
			kind: C("press_key"),
			element_index: K.nullable(),
			key: i(),
			duration: u().finite().min(0).max(30).optional()
		}),
		n({
			kind: C("scroll"),
			target: ls,
			direction: S([
				"up",
				"down",
				"left",
				"right",
				"u",
				"d",
				"l",
				"r"
			]),
			pages: u().optional()
		}),
		n({
			kind: C("select_text"),
			element_index: K,
			text: i(),
			prefix: i().optional(),
			suffix: i().optional(),
			selection_type: S([
				"text",
				"cursor_before",
				"cursor_after"
			]).optional()
		}),
		n({
			kind: C("set_value"),
			element_index: K,
			value: i()
		}),
		n({
			kind: C("type_text"),
			element_index: K.nullable(),
			text: i()
		})
	]), hs = n({
		browser_id: i(),
		tab_id: i(),
		action: us
	}), cu = n({}), ms = "tab_ax_action";
	function ps(e) {
		return new h(ms, hs, e);
	}
	var du = l({
		PayloadSchema: () => fs,
		ResultSchema: () => lu,
		commandType: () => ys,
		create: () => Pe
	});
	const fs = n({
		browser_id: i(),
		selector: i().optional(),
		selector_mode: C("all").optional(),
		tab_id: i(),
		script: i(),
		timeout_ms: u().int().positive().optional()
	}), lu = n({ value: Z().optional() }), ys = "playwright_evaluate";
	function Pe(e) {
		return new h(ys, fs, e);
	}
	var uu = l({
		PayloadSchema: () => _s,
		ResultSchema: () => hu,
		commandType: () => bs,
		create: () => gs
	});
	const _s = n({
		browser_id: i(),
		tab_id: i(),
		x: u(),
		y: u(),
		button: u().optional(),
		keys: g(i()).optional()
	}), hu = n({}), bs = "cua_click";
	function gs(e) {
		return new h(bs, _s, e);
	}
	var mu = l({
		PayloadSchema: () => ws,
		ResultSchema: () => pu,
		commandType: () => vs,
		create: () => Ss
	});
	const ws = n({
		browser_id: i(),
		tab_id: i(),
		x: u(),
		y: u(),
		timeout_ms: u().int().positive().optional()
	}), pu = n({}), vs = "cua_download_media";
	function Ss(e) {
		return new h(vs, ws, e);
	}
	var fu = l({
		PayloadSchema: () => xs,
		ResultSchema: () => yu,
		commandType: () => Ts,
		create: () => $s
	});
	const xs = n({
		browser_id: i(),
		tab_id: i(),
		x: u(),
		y: u(),
		keys: g(i()).optional()
	}), yu = n({}), Ts = "cua_double_click";
	function $s(e) {
		return new h(Ts, xs, e);
	}
	var _u = l({
		PayloadSchema: () => ks,
		ResultSchema: () => bu,
		commandType: () => Rs,
		create: () => Cs
	});
	const ks = n({
		browser_id: i(),
		tab_id: i(),
		keys: g(i())
	}), bu = n({}), Rs = "cua_keypress";
	function Cs(e) {
		return new h(Rs, ks, e);
	}
	var gu = l({
		PayloadSchema: () => Is,
		ResultSchema: () => wu,
		commandType: () => Ps,
		create: () => Es
	});
	const Is = n({
		browser_id: i(),
		tab_id: i(),
		path: g(n({
			x: u(),
			y: u()
		})),
		keys: g(i()).optional()
	}), wu = n({}), Ps = "cua_drag";
	function Es(e) {
		return new h(Ps, Is, e);
	}
	var vu = l({
		PayloadSchema: () => As,
		ResultSchema: () => Su,
		commandType: () => Ns,
		create: () => Ds
	});
	const As = n({
		browser_id: i(),
		tab_id: i(),
		x: u(),
		y: u(),
		keys: g(i()).optional()
	}), Su = n({}), Ns = "cua_move";
	function Ds(e) {
		return new h(Ns, As, e);
	}
	var xu = l({
		PayloadSchema: () => Os,
		ResultSchema: () => Tu,
		commandType: () => Ms,
		create: () => Bs
	});
	const Os = n({
		browser_id: i(),
		tab_id: i(),
		x: u(),
		y: u(),
		scroll_x: u(),
		scroll_y: u(),
		keys: g(i()).optional()
	}), Tu = n({}), Ms = "cua_scroll";
	function Bs(e) {
		return new h(Ms, Os, e);
	}
	var $u = l({
		PayloadSchema: () => js,
		ResultSchema: () => ku,
		commandType: () => Ls,
		create: () => Zs
	});
	const js = n({
		browser_id: i(),
		tab_id: i(),
		text: i()
	}), ku = n({}), Ls = "cua_type";
	function Zs(e) {
		return new h(Ls, js, e);
	}
	const te = i({
		required_error: "node_id is required",
		invalid_type_error: "node_id must be a string"
	}).min(1, "node_id must not be empty");
	var Ru = l({
		DomNodeIdSchema: () => te,
		PayloadSchema: () => qs,
		ResultSchema: () => Cu,
		commandType: () => Us,
		create: () => Fs
	});
	const qs = n({
		browser_id: i(),
		tab_id: i(),
		node_id: te
	}), Cu = n({}), Us = "dom_cua_click";
	function Fs(e) {
		return new h(Us, qs, e);
	}
	var Iu = l({
		PayloadSchema: () => Vs,
		ResultSchema: () => Pu,
		commandType: () => Hs,
		create: () => Ws
	});
	const Vs = n({
		browser_id: i(),
		tab_id: i(),
		node_id: te,
		timeout_ms: u().int().positive().optional()
	}), Pu = n({}), Hs = "dom_cua_download_media";
	function Ws(e) {
		return new h(Hs, Vs, e);
	}
	var Eu = l({
		PayloadSchema: () => zs,
		ResultSchema: () => Au,
		commandType: () => Gs,
		create: () => Js
	});
	const zs = n({
		browser_id: i(),
		tab_id: i(),
		node_id: te
	}), Au = n({}), Gs = "dom_cua_double_click";
	function Js(e) {
		return new h(Gs, zs, e);
	}
	var Nu = l({
		PayloadSchema: () => Ks,
		ResultSchema: () => Du,
		commandType: () => Ys,
		create: () => Xs
	});
	const Ks = n({
		browser_id: i(),
		tab_id: i()
	}), Du = Z(), Ys = "dom_cua_get_visible_dom";
	function Xs(e) {
		return new h(Ys, Ks, e);
	}
	var Ou = l({
		PayloadSchema: () => Qs,
		ResultSchema: () => Mu,
		commandType: () => ei,
		create: () => ti
	});
	const Qs = n({
		browser_id: i(),
		tab_id: i(),
		keys: g(i())
	}), Mu = n({}), ei = "dom_cua_keypress";
	function ti(e) {
		return new h(ei, Qs, e);
	}
	var Bu = l({
		PayloadSchema: () => ai,
		ResultSchema: () => ju,
		commandType: () => ri,
		create: () => si
	});
	const ai = n({
		browser_id: i(),
		tab_id: i(),
		scroll_x: u(),
		scroll_y: u(),
		node_id: te.optional()
	}), ju = n({}), ri = "dom_cua_scroll";
	function si(e) {
		return new h(ri, ai, e);
	}
	var Lu = l({
		PayloadSchema: () => ii,
		ResultSchema: () => Zu,
		commandType: () => ni,
		create: () => oi
	});
	const ii = n({
		browser_id: i(),
		tab_id: i(),
		text: i()
	}), Zu = n({}), ni = "dom_cua_type";
	function oi(e) {
		return new h(ni, ii, e);
	}
	var qu = l({
		PayloadSchema: () => ci,
		ResultSchema: () => Uu,
		commandType: () => di,
		create: () => li
	});
	const ci = n({
		browser_id: i(),
		tab_id: i(),
		selector: i(),
		modifiers: g(S([
			"Alt",
			"Control",
			"ControlOrMeta",
			"Meta",
			"Shift"
		])).optional(),
		button: S([
			"left",
			"right",
			"middle"
		]).optional(),
		force: k().optional(),
		timeout_ms: u().int().positive().optional()
	}), Uu = n({}), di = "playwright_locator_click";
	function li(e) {
		return new h(di, ci, e);
	}
	var Fu = l({
		PayloadSchema: () => ui,
		ResultSchema: () => Vu,
		commandType: () => hi,
		create: () => mi
	});
	const ui = n({
		browser_id: i(),
		tab_id: i(),
		selector: i(),
		modifiers: g(S([
			"Alt",
			"Control",
			"ControlOrMeta",
			"Meta",
			"Shift"
		])).optional(),
		button: S([
			"left",
			"right",
			"middle"
		]).optional(),
		force: k().optional(),
		timeout_ms: u().int().positive().optional()
	}), Vu = n({}), hi = "playwright_locator_dblclick";
	function mi(e) {
		return new h(hi, ui, e);
	}
	var Hu = l({
		PayloadSchema: () => pi,
		ResultSchema: () => Wu,
		commandType: () => fi,
		create: () => yi
	});
	const pi = n({
		browser_id: i(),
		tab_id: i(),
		selector: i(),
		timeout_ms: u().int().positive().optional()
	}), Wu = n({ path: i() }), fi = "playwright_locator_download_media";
	function yi(e) {
		return new h(fi, pi, e);
	}
	var zu = l({
		PayloadSchema: () => _i,
		ResultSchema: () => Gu,
		commandType: () => bi,
		create: () => ct
	});
	const _i = n({
		browser_id: i(),
		tab_id: i(),
		selector: i(),
		value: i(),
		replace: k(),
		timeout_ms: u().int().positive().optional()
	}), Gu = n({}), bi = "playwright_locator_fill";
	function ct(e) {
		return new h(bi, _i, e);
	}
	var Ju = l({
		PayloadSchema: () => gi,
		ResultSchema: () => Ku,
		commandType: () => wi,
		create: () => vi
	});
	const gi = n({
		browser_id: i(),
		tab_id: i(),
		selector: i(),
		value: i(),
		timeout_ms: u().int().positive().optional()
	}), Ku = n({}), wi = "playwright_locator_press";
	function vi(e) {
		return new h(wi, gi, e);
	}
	var Yu = l({
		PayloadSchema: () => Si,
		ResultSchema: () => Xu,
		commandType: () => xi,
		create: () => Ti
	});
	const Si = n({
		browser_id: i(),
		tab_id: i(),
		selector: i(),
		value: i(),
		timeout_ms: u().int().positive().optional()
	}), Xu = n({}), xi = "playwright_locator_press_sequentially";
	function Ti(e) {
		return new h(xi, Si, e);
	}
	var Qu = l({
		PayloadSchema: () => $i,
		ResultSchema: () => eh,
		commandType: () => ki,
		create: () => Ri
	});
	const $i = n({
		browser_id: i(),
		tab_id: i(),
		selector: i(),
		state: S([
			"attached",
			"detached",
			"visible",
			"hidden"
		]),
		timeout_ms: u().int().positive().optional()
	}), eh = n({}), ki = "playwright_locator_wait_for";
	function Ri(e) {
		return new h(ki, $i, e);
	}
	var th = l({
		PayloadSchema: () => Ci,
		ResultSchema: () => ah,
		commandType: () => Ii,
		create: () => Pi
	});
	const Ci = n({
		browser_id: i(),
		tab_id: i(),
		selector: i()
	}), ah = n({ count: u().int() }), Ii = "playwright_locator_count";
	function Pi(e) {
		return new h(Ii, Ci, e);
	}
	var rh = l({
		PayloadSchema: () => Ai,
		ResultSchema: () => sh,
		SelectOptionSchema: () => Ei,
		commandType: () => Ni,
		create: () => Di
	});
	const Ei = n({
		value: i().optional(),
		label: i().optional(),
		index: u().int().nonnegative().optional()
	}).superRefine((e, t) => {
		e.value === void 0 && e.label === void 0 && e.index === void 0 && t.addIssue({
			code: m.custom,
			message: "Select option requires value, label, or index"
		});
	}), Ai = n({
		browser_id: i(),
		tab_id: i(),
		selector: i(),
		selections: g(Ei).min(1),
		timeout_ms: u().int().positive().optional()
	}), sh = n({}), Ni = "playwright_locator_select_option";
	function Di(e) {
		return new h(Ni, Ai, e);
	}
	var ih = l({
		PayloadSchema: () => Oi,
		ResultSchema: () => nh,
		commandType: () => Mi,
		create: () => Bi
	});
	const Oi = n({
		browser_id: i(),
		tab_id: i(),
		selector: i(),
		checked: k(),
		force: k().optional(),
		timeout_ms: u().int().positive().optional()
	}), nh = n({}), Mi = "playwright_locator_set_checked";
	function Bi(e) {
		return new h(Mi, Oi, e);
	}
	var oh = l({
		PayloadSchema: () => ji,
		ResultSchema: () => ch,
		commandType: () => Li,
		create: () => Zi
	});
	const ji = n({
		browser_id: i(),
		tab_id: i(),
		selector: i()
	}), ch = n({ value: k() }), Li = "playwright_locator_is_visible";
	function Zi(e) {
		return new h(Li, ji, e);
	}
	var dh = l({
		PayloadSchema: () => qi,
		ResultSchema: () => lh,
		commandType: () => Ui,
		create: () => Fi
	});
	const qi = n({
		browser_id: i(),
		tab_id: i(),
		selector: i()
	}), lh = n({ value: k() }), Ui = "playwright_locator_is_enabled";
	function Fi(e) {
		return new h(Ui, qi, e);
	}
	var uh = l({
		PayloadSchema: () => Vi,
		ResultSchema: () => hh,
		commandType: () => Hi,
		create: () => Wi
	});
	const Vi = n({
		browser_id: i(),
		tab_id: i(),
		selector: i(),
		timeout_ms: u().int().positive().optional()
	}), hh = n({ values: g(i()) }), Hi = "playwright_locator_all_text_contents";
	function Wi(e) {
		return new h(Hi, Vi, e);
	}
	var mh = l({
		PayloadSchema: () => zi,
		ResultSchema: () => ph,
		commandType: () => Gi,
		create: () => Ji
	});
	const zi = n({
		browser_id: i(),
		tab_id: i(),
		selector: i(),
		timeout_ms: u().int().positive().optional()
	}), ph = n({ value: i().nullable() }), Gi = "playwright_locator_text_content";
	function Ji(e) {
		return new h(Gi, zi, e);
	}
	var fh = l({
		PayloadSchema: () => Ki,
		ResultSchema: () => yh,
		commandType: () => Yi,
		create: () => Xi
	});
	const Ki = n({
		browser_id: i(),
		tab_id: i(),
		selector: i(),
		timeout_ms: u().int().positive().optional()
	}), yh = n({ value: i() }), Yi = "playwright_locator_inner_text";
	function Xi(e) {
		return new h(Yi, Ki, e);
	}
	var _h = l({
		PayloadSchema: () => Qi,
		ResultSchema: () => bh,
		commandType: () => en,
		create: () => tn
	});
	const Qi = n({
		browser_id: i(),
		tab_id: i(),
		selector: i(),
		name: i(),
		timeout_ms: u().int().positive().optional()
	}), bh = n({ value: i().nullable() }), en = "playwright_locator_get_attribute";
	function tn(e) {
		return new h(en, Qi, e);
	}
	var gh = l({
		PayloadSchema: () => an,
		ResultSchema: () => vh,
		commandType: () => rn,
		create: () => sn
	});
	const an = n({
		browser_id: i(),
		tab_id: i(),
		selector: i(),
		relative_selector: i().optional(),
		timeout_ms: u().int().positive().optional()
	}), vh = n({ values: g(n({
		attributes: de(i()),
		inner_text: i(),
		text_content: i().nullable()
	}).nullable()) }), rn = "playwright_locator_read_all";
	function sn(e) {
		return new h(rn, an, e);
	}
	var Sh = l({
		PayloadSchema: () => nn,
		ResultSchema: () => Th,
		commandType: () => on,
		create: () => cn
	});
	const xh = S([
		"load",
		"domcontentloaded",
		"networkidle",
		"commit"
	]), nn = n({
		browser_id: i(),
		tab_id: i(),
		url: i(),
		wait_until: xh.optional(),
		timeout_ms: u().int().positive().optional()
	}), Th = n({ url: i().optional() }), on = "playwright_wait_for_url";
	function cn(e) {
		return new h(on, nn, e);
	}
	var $h = l({
		LoadStateSchema: () => dn,
		PayloadSchema: () => ln,
		ResultSchema: () => kh,
		commandType: () => un,
		create: () => hn
	});
	const dn = S([
		"load",
		"domcontentloaded",
		"networkidle"
	]), ln = n({
		browser_id: i(),
		tab_id: i(),
		state: dn.optional(),
		timeout_ms: u().int().positive().optional()
	}), kh = n({}), un = "playwright_wait_for_load_state";
	function hn(e) {
		return new h(un, ln, e);
	}
	var Rh = l({
		PayloadSchema: () => mn,
		ResultSchema: () => Ch,
		commandType: () => pn,
		create: () => fn
	});
	const mn = n({
		browser_id: i(),
		tab_id: i(),
		timeout_ms: u().int().nonnegative()
	}), Ch = n({}), pn = "playwright_wait_for_timeout";
	function fn(e) {
		return new h(pn, mn, e);
	}
	var Ih = l({
		PayloadSchema: () => yn,
		ResultSchema: () => Ph,
		commandType: () => _n,
		create: () => bn
	});
	const yn = n({
		browser_id: i(),
		tab_id: i()
	}), Ph = n({
		dom_snapshot: i(),
		snapshotId: i().optional(),
		siwc: Re.optional()
	}), _n = "playwright_dom_snapshot";
	function bn(e) {
		return new h(_n, yn, e);
	}
	var Eh = l({
		PayloadSchema: () => gn,
		ResultSchema: () => Oh,
		commandType: () => wn,
		create: () => vn
	});
	const Ah = n({
		x: u(),
		y: u(),
		width: u(),
		height: u()
	}), Nh = n({
		primary: i().nullable().optional(),
		candidates: g(i()),
		frameSelectors: g(i()).optional()
	}), Dh = n({
		nodeId: u().int().positive().nullable().optional(),
		tagName: i(),
		role: i().nullable().optional(),
		visibleText: i().nullable().optional(),
		ariaName: i().nullable().optional(),
		testId: i().nullable().optional(),
		boundingBox: Ah.nullable().optional(),
		preview: i(),
		selector: Nh
	}), gn = n({
		browser_id: i(),
		tab_id: i(),
		x: u(),
		y: u(),
		include_non_interactable: k().optional()
	}), Oh = g(Dh), wn = "playwright_element_info";
	function vn(e) {
		return new h(wn, gn, e);
	}
	var Mh = l({
		PayloadSchema: () => Sn,
		ResultSchema: () => Bh,
		commandType: () => xn,
		create: () => Tn
	});
	const Sn = n({
		browser_id: i(),
		tab_id: i(),
		x: u(),
		y: u(),
		include_non_interactable: k().optional()
	}), Bh = n({ data: i() }), xn = "playwright_element_screenshot";
	function Tn(e) {
		return new h(xn, Sn, e);
	}
	var jh = l({
		PayloadSchema: () => $n,
		ResultSchema: () => Lh,
		commandType: () => kn,
		create: () => Rn
	});
	const $n = n({
		browser_id: i(),
		tab_id: i(),
		timeout_ms: u().int().positive().optional()
	}), Lh = n({ download_id: i() }), kn = "playwright_wait_for_download";
	function Rn(e) {
		return new h(kn, $n, e);
	}
	var Zh = l({
		PayloadSchema: () => Cn,
		ResultSchema: () => qh,
		commandType: () => In,
		create: () => Pn
	});
	const Cn = n({
		browser_id: i(),
		tab_id: i(),
		download_id: i(),
		timeout_ms: u().int().positive().optional()
	}), qh = n({ path: i().nullable() }), In = "playwright_download_path";
	function Pn(e) {
		return new h(In, Cn, e);
	}
	var Uh = l({
		PayloadSchema: () => En,
		ResultSchema: () => Fh,
		commandType: () => An,
		create: () => Nn
	});
	const En = n({
		browser_id: i(),
		tab_id: i(),
		timeout_ms: u().int().positive().optional()
	}), Fh = n({
		file_chooser_id: i(),
		is_multiple: k()
	}), An = "playwright_wait_for_file_chooser";
	function Nn(e) {
		return new h(An, En, e);
	}
	var Vh = l({
		PayloadSchema: () => Dn,
		ResultSchema: () => Hh,
		commandType: () => On,
		create: () => Mn
	});
	const Dn = n({
		browser_id: i(),
		tab_id: i(),
		file_chooser_id: i(),
		files: g(i()),
		timeout_ms: u().int().positive().optional()
	}), Hh = n({}), On = "playwright_file_chooser_set_files";
	function Mn(e) {
		return new h(On, Dn, e);
	}
	var Wh = l({
		AtlasCommand: () => h,
		BrowserUserClaimTab: () => kl,
		BrowserUserGetTabContext: () => Cl,
		BrowserUserHistory: () => El,
		BrowserUserOpenTabs: () => Il,
		CloseTab: () => Wd,
		CreateTab: () => Ud,
		CuaClick: () => uu,
		CuaDoubleClick: () => fu,
		CuaDownloadMedia: () => mu,
		CuaDrag: () => gu,
		CuaKeypress: () => _u,
		CuaMove: () => vu,
		CuaScroll: () => xu,
		CuaType: () => $u,
		DomCuaClick: () => Ru,
		DomCuaDoubleClick: () => Eu,
		DomCuaDownloadMedia: () => Iu,
		DomCuaGetVisibleDom: () => Nu,
		DomCuaKeypress: () => Ou,
		DomCuaScroll: () => Bu,
		DomCuaType: () => Lu,
		GetBrowser: () => Ad,
		GetBrowserDocumentation: () => jd,
		GetBrowserForUrl: () => Zd,
		GetDefaultBrowser: () => Dd,
		GetDocumentation: () => Md,
		GetTab: () => Vd,
		ListBrowsers: () => Pd,
		ListTabs: () => dl,
		ManualHandoff: () => Kd,
		MarkTab: () => Gd,
		NameSession: () => il,
		NavigateTab: () => bl,
		NavigateTabBack: () => wl,
		NavigateTabForward: () => Sl,
		NavigateTabReload: () => Tl,
		PlaywrightDomSnapshot: () => Ih,
		PlaywrightDownloadPath: () => Zh,
		PlaywrightElementInfo: () => Eh,
		PlaywrightElementScreenshot: () => Mh,
		PlaywrightEvaluate: () => du,
		PlaywrightFileChooserSetFiles: () => Vh,
		PlaywrightLocatorAllTextContents: () => uh,
		PlaywrightLocatorClick: () => qu,
		PlaywrightLocatorCount: () => th,
		PlaywrightLocatorDblclick: () => Fu,
		PlaywrightLocatorDownloadMedia: () => Hu,
		PlaywrightLocatorFill: () => zu,
		PlaywrightLocatorGetAttribute: () => _h,
		PlaywrightLocatorInnerText: () => fh,
		PlaywrightLocatorIsEnabled: () => dh,
		PlaywrightLocatorIsVisible: () => oh,
		PlaywrightLocatorPress: () => Ju,
		PlaywrightLocatorPressSequentially: () => Yu,
		PlaywrightLocatorReadAll: () => gh,
		PlaywrightLocatorSelectOption: () => rh,
		PlaywrightLocatorSetChecked: () => ih,
		PlaywrightLocatorTextContent: () => mh,
		PlaywrightLocatorWaitFor: () => Qu,
		PlaywrightWaitForDownload: () => jh,
		PlaywrightWaitForFileChooser: () => Uh,
		PlaywrightWaitForLoadState: () => $h,
		PlaywrightWaitForTimeout: () => Rh,
		PlaywrightWaitForURL: () => Sh,
		RuntimeConfig: () => al,
		SelectedTab: () => ol,
		SiwcAuthenticate: () => Xd,
		SiwcDetect: () => el,
		TabAxAction: () => ou,
		TabAxGetState: () => iu,
		TabClipboardRead: () => Jl,
		TabClipboardReadText: () => Hl,
		TabClipboardWrite: () => Ql,
		TabClipboardWriteText: () => zl,
		TabContentExport: () => Ll,
		TabContentExportGSuite: () => ql,
		TabContentExportYouTubeTranscript: () => Fl,
		TabDevLogs: () => Nl,
		TabGetJsDialog: () => Ol,
		TabHandleJsDialog: () => Bl,
		TabScreenshot: () => ru,
		TabsContent: () => ul,
		WebMcpInvokeTool: () => yl,
		WebMcpListTools: () => fl
	}), Bn = class {
		#e;
		#a;
		#t;
		constructor({ createBrowser: e, onBrowserUsed: t, transport: a }) {
			this.#e = e, this.#a = t, this.#t = a;
		}
		async list() {
			return await this.#t.send({ command: Kt() });
		}
		async get(e) {
			if (!e) throw new Error("browsers.get requires a browser id");
			const t = await this.#t.send({ command: Qt({ id: e }) });
			return this.#r(t);
		}
		async getDefault() {
			const e = await this.#t.send({ command: aa() });
			return this.#r(e);
		}
		async getForUrl(e) {
			const t = await this.#t.send({ command: ua({ url: e }) });
			return this.#r(t);
		}
		#r(e) {
			return this.#a?.(e), this.#e({
				browserInfo: e,
				onBrowserUsed: () => this.#a?.(e),
				transport: this.#t
			});
		}
	}, dt = class {
		#e;
		constructor(e) {
			this.#e = e;
		}
		async get(e) {
			return await this.#e.send({ command: ia({ name: e }) });
		}
	}, jn = class {
		constructor({ createBrowser: e, onBrowserUsed: t, transport: a }) {
			if (!a) throw new Error("Agent requires a transport instance");
			this.documentation = new dt(a), this.browsers = new Bn({
				createBrowser: e,
				onBrowserUsed: t,
				transport: a
			});
		}
	}, Ln = class {
		#e;
		#a;
		#t;
		constructor({ browserId: e, createTab: t, transport: a }) {
			this.#e = e, this.#a = t, this.#t = a;
		}
		async new() {
			const e = await this.#t.send({ command: pa({ browser_id: this.#e }) });
			return this.#a(e);
		}
		async selected() {
			const e = await this.#t.send({ command: Ua({ browser_id: this.#e }) });
			if (e.id) return this.#a(e);
		}
		async list() {
			return (await this.#t.send({ command: Ha({ browser_id: this.#e }) })).tabs;
		}
		async content(e) {
			const t = Ja({
				browser_id: this.#e,
				urls: e.urls,
				content_type: e.contentType,
				...e.timeoutMs === void 0 ? {} : { timeout_ms: e.timeoutMs }
			});
			return t.parse(), e.urls.length === 0 ? [] : (await this.#t.send({ command: t })).results;
		}
		async get(e) {
			if (!e) throw new Error("tabs.get requires a tab id");
			const t = await this.#t.send({ command: rt({
				browser_id: this.#e,
				tab_id: e
			}) });
			return this.#a(t);
		}
	};
	const zh = () => \`req-\${Date.now().toString(16)}-\${Math.random().toString(16).slice(2)}\`, Ee = (e) => {
		const t = atob(e), a = new Uint8Array(t.length);
		for (let r = 0; r < t.length; r += 1) a[r] = t.charCodeAt(r);
		return a;
	};
	var Zn = class {
		#e;
		#a;
		#t;
		constructor({ browserId: e, createTab: t, transport: a }) {
			this.#e = e, this.#a = t, this.#t = a;
		}
		async openTabs() {
			return (await this.#t.send({ command: Sr({ browser_id: this.#e }) })).tabs;
		}
		async getTabContext(e) {
			const t = await this.#t.send({ command: gr({
				browser_id: this.#e,
				tab_id: e === void 0 ? void 0 : qn(e, "getTabContext")
			}) }), a = _r.parse(t);
			return a.kind !== "document" ? a : {
				data: Ee(a.dataBase64),
				fileName: a.fileName,
				kind: a.kind,
				mimeType: a.mimeType,
				title: a.title,
				url: a.url
			};
		}
		async claimTab(e) {
			const t = qn(e, "claimTab"), a = await this.#t.send({ command: fr({
				browser_id: this.#e,
				tab_id: t
			}) });
			return this.#a(a);
		}
	};
	const qn = (e, t) => {
		if (typeof e == "string") {
			if (e.length === 0) throw new Error(\`browser.user.\${t} received an empty tab id\`);
			return e;
		}
		if (e && typeof e == "object" && typeof e.id == "string") return e.id;
		throw new Error(\`browser.user.\${t} expects a tab returned by browser.user.openTabs() or a tab id\`);
	}, Un = Object.getOwnPropertyDescriptor(RegExp.prototype, "source")?.get;
	function Ae(e) {
		if (e == null || typeof e != "object" || !Un) return !1;
		try {
			return Reflect.apply(Un, e, []), !0;
		} catch {
			return !1;
		}
	}
	function Fn(e) {
		if (e == null || typeof e != "object") return !1;
		const t = Object.getPrototypeOf(e);
		return t === null || Object.getPrototypeOf(t) === null;
	}
	function Gh(e, t) {
		if (e == null || typeof e != "object") return !1;
		try {
			return "message" in e && e.message === t;
		} catch {
			return !1;
		}
	}
	function Ne(e, t) {
		const a = e instanceof Error ? e.message : String(e), r = /* @__PURE__ */ new Error(\`\${a}
\${t}\`);
		return e instanceof Error && e.stack && (r.stack = \`\${r.name}: \${r.message}
\${e.stack}\`), r;
	}
	function Jh(e, t) {
		const a = e instanceof Error ? e.message : String(e);
		return /strict mode violation/i.test(a) ? "multiple_matches" : /intercept|receives pointer events/i.test(a) ? "intercepted" : t.matchCount === 0 ? "no_matches" : t.visibleCount === 0 ? "no_visible_match" : "action_failed";
	}
	function Kh(e, t) {
		const a = Hn(t);
		if (typeof e == "string") {
			if (e.length === 0) throw new Error("playwright.evaluate requires a pageFunction");
			return \`const arg = \${a};
return (\${e});\`;
		}
		if (typeof e == "function") return [
			\`const arg = \${a};\`,
			\`const __playwrightEvaluate = (\${e.toString()});\`,
			"return await __playwrightEvaluate(arg);"
		].join(\`
\`);
		throw new Error("playwright.evaluate requires a string or function");
	}
	function Vn(e, t, a) {
		const r = Hn(t), s = a === "all" ? "locator.evaluateAll" : "locator.evaluate";
		if (typeof e == "string") {
			if (e.length === 0) throw new Error(\`\${s} requires a pageFunction\`);
			return \`const arg = \${r};
return (\${e});\`;
		}
		if (typeof e == "function") return [
			\`const arg = \${r};\`,
			\`const __playwrightEvaluate = (\${e.toString()});\`,
			\`return await __playwrightEvaluate(\${a === "all" ? "elements" : "element"}, arg);\`
		].join(\`
\`);
		throw new Error(\`\${s} requires a string or function\`);
	}
	function Hn(e) {
		if (e === void 0) return "undefined";
		try {
			const t = JSON.stringify(e);
			if (t !== void 0) return t;
		} catch (t) {
			throw Ne(t, "playwright.evaluate arg must be JSON-serializable");
		}
		throw new Error("playwright.evaluate arg must be JSON-serializable");
	}
	function Wn(e, t) {
		const { x: a, y: r } = t;
		if (!Number.isFinite(a) || !Number.isFinite(r)) throw new Error(\`\${e} requires numeric x and y coordinates\`);
		return {
			x: a,
			y: r
		};
	}
	function Yh(e) {
		const t = Array.isArray(e) ? e : [e];
		if (!t.length) throw new Error("locator.selectOption requires at least one value");
		return t.map((a) => {
			if (typeof a == "string") return { value: a };
			if (!a || typeof a != "object") throw new Error("locator.selectOption requires a string or { value?, label?, index? }");
			const r = {};
			if (a.value !== void 0) {
				if (typeof a.value != "string") throw new Error("locator.selectOption value must be a string");
				r.value = a.value;
			}
			if (a.label !== void 0) {
				if (typeof a.label != "string") throw new Error("locator.selectOption label must be a string");
				r.label = a.label;
			}
			if (a.index !== void 0) {
				if (!Number.isInteger(a.index) || a.index < 0) throw new Error("locator.selectOption index must be a non-negative integer");
				r.index = a.index;
			}
			if (r.value === void 0 && r.label === void 0 && r.index === void 0) throw new Error("locator.selectOption requires value, label, or index for each selection");
			return r;
		});
	}
	const zn = Symbol("browserAuthSelectorDetails");
	var ue = class I {
		#e;
		#a;
		#t;
		#r;
		#s;
		#i;
		#n;
		constructor({ browserId: t, tabId: a, selector: r, transport: s, collectionReadCache: c, collectionReadIndex: o, collectionRelativeSelector: d }) {
			this.#e = t, this.#a = a, this.#t = r, this.#r = s, this.#s = c, this.#i = o, this.#n = d;
		}
		static browserAuthSelector(t) {
			return t[zn]();
		}
		[zn]() {
			return {
				browserId: this.#e,
				selector: this.#t,
				tabId: this.#a
			};
		}
		async click(t = {}) {
			try {
				await this.#r.send({
					command: li({
						browser_id: this.#e,
						tab_id: this.#a,
						selector: this.#t,
						modifiers: t.modifiers,
						button: t.button,
						force: t.force,
						timeout_ms: t.timeoutMs
					}),
					timeoutMs: t.timeoutMs
				}), this.#s?.clear();
			} catch (a) {
				throw await this.actionError(a, "click", \`waiting on click for selector \${this.#t}\`);
			}
		}
		async dblclick(t = {}) {
			try {
				await this.#r.send({
					command: mi({
						browser_id: this.#e,
						tab_id: this.#a,
						selector: this.#t,
						modifiers: t.modifiers,
						button: t.button,
						force: t.force,
						timeout_ms: t.timeoutMs
					}),
					timeoutMs: t.timeoutMs
				}), this.#s?.clear();
			} catch (a) {
				throw await this.actionError(a, "dblclick", \`waiting on dblclick for selector \${this.#t}\`);
			}
		}
		async selectOption(t, { timeoutMs: a } = {}) {
			const r = Yh(t);
			try {
				await this.#r.send({
					command: Di({
						browser_id: this.#e,
						tab_id: this.#a,
						selector: this.#t,
						selections: r,
						timeout_ms: a
					}),
					timeoutMs: a
				}), this.#s?.clear();
			} catch (s) {
				throw await this.actionError(s, "selectOption", \`locator.selectOption failed for selector \${this.#t}\`);
			}
		}
		async fill(t, { timeoutMs: a } = {}) {
			if (t == null) throw new Error("locator.fill requires a value");
			try {
				await this.#r.send({
					command: ct({
						browser_id: this.#e,
						tab_id: this.#a,
						selector: this.#t,
						value: t,
						replace: !0,
						timeout_ms: a
					}),
					timeoutMs: a
				}), this.#s?.clear();
			} catch (r) {
				throw await this.actionError(r, "fill", \`locator.fill failed for selector \${this.#t}\`);
			}
		}
		async type(t, { timeoutMs: a } = {}) {
			if (t == null) throw new Error("locator.type requires a value");
			try {
				await this.#r.send({
					command: ct({
						browser_id: this.#e,
						tab_id: this.#a,
						selector: this.#t,
						value: t,
						replace: !1,
						timeout_ms: a
					}),
					timeoutMs: a
				}), this.#s?.clear();
			} catch (r) {
				throw await this.actionError(r, "type", \`locator.type failed for selector \${this.#t}\`);
			}
		}
		async pressSequentially(t, a = {}) {
			if (t == null) throw new Error("locator.pressSequentially requires a value");
			try {
				await this.#r.send({
					command: Ti({
						browser_id: this.#e,
						tab_id: this.#a,
						selector: this.#t,
						value: t,
						...a.timeoutMs == null ? {} : { timeout_ms: a.timeoutMs }
					}),
					timeoutMs: a.timeoutMs
				}), this.#s?.clear();
			} catch (r) {
				throw await this.actionError(r, "pressSequentially", \`locator.pressSequentially failed for selector \${this.#t}\`);
			}
		}
		async press(t, { timeoutMs: a } = {}) {
			if (t == null) throw new Error("locator.press requires a value");
			try {
				await this.#r.send({
					command: vi({
						browser_id: this.#e,
						tab_id: this.#a,
						selector: this.#t,
						value: t,
						timeout_ms: a
					}),
					timeoutMs: a
				}), this.#s?.clear();
			} catch (r) {
				throw await this.actionError(r, "press", \`locator.press failed for selector \${this.#t}\`);
			}
		}
		async setChecked(t, a = {}) {
			if (typeof t != "boolean") throw new Error("locator.setChecked requires a boolean");
			try {
				await this.#r.send({
					command: Bi({
						browser_id: this.#e,
						tab_id: this.#a,
						selector: this.#t,
						checked: t,
						force: a.force,
						timeout_ms: a.timeoutMs
					}),
					timeoutMs: a.timeoutMs
				}), this.#s?.clear();
			} catch (r) {
				throw await this.actionError(r, "setChecked", \`locator.setChecked(\${t}) failed for selector \${this.#t}\`);
			}
		}
		async check(t = {}) {
			await this.setChecked(!0, t);
		}
		async uncheck(t = {}) {
			await this.setChecked(!1, t);
		}
		async waitFor({ state: t, timeoutMs: a }) {
			if (!t) throw new Error("locator.waitFor requires a state");
			try {
				await this.#r.send({
					command: Ri({
						browser_id: this.#e,
						tab_id: this.#a,
						selector: this.#t,
						state: t,
						timeout_ms: a
					}),
					timeoutMs: a
				});
			} catch (r) {
				throw await this.actionError(r, "waitFor", \`locator.waitFor(\${t}) timed out for selector \${this.#t}\`);
			}
		}
		async count() {
			return (await this.#r.send({ command: Pi({
				browser_id: this.#e,
				tab_id: this.#a,
				selector: this.#t
			}) })).count;
		}
		async all() {
			const t = await this.count(), a = new Xh({
				browserId: this.#e,
				tabId: this.#a,
				selector: this.#t,
				transport: this.#r
			});
			return Array.from({ length: t }, (r, s) => new I({
				browserId: this.#e,
				tabId: this.#a,
				selector: \`\${this.#t} >> nth=\${s}\`,
				transport: this.#r,
				collectionReadCache: a,
				collectionReadIndex: s
			}));
		}
		async textContent({ timeoutMs: t } = {}) {
			const a = await this.cachedRead(t);
			return a ? a.text_content : (await this.#r.send({
				command: Ji({
					browser_id: this.#e,
					tab_id: this.#a,
					selector: this.#t,
					timeout_ms: t
				}),
				timeoutMs: t
			})).value;
		}
		async innerText({ timeoutMs: t } = {}) {
			const a = await this.cachedRead(t);
			return a ? a.inner_text : (await this.#r.send({
				command: Xi({
					browser_id: this.#e,
					tab_id: this.#a,
					selector: this.#t,
					timeout_ms: t
				}),
				timeoutMs: t
			})).value;
		}
		async getAttribute(t, { timeoutMs: a } = {}) {
			if (!t) throw new Error("locator.getAttribute requires a name");
			const r = await this.cachedRead(a);
			return r ? Object.prototype.hasOwnProperty.call(r.attributes, t) ? r.attributes[t] ?? null : null : (await this.#r.send({
				command: tn({
					browser_id: this.#e,
					tab_id: this.#a,
					selector: this.#t,
					name: t,
					timeout_ms: a
				}),
				timeoutMs: a
			})).value;
		}
		async isVisible() {
			return (await this.#r.send({ command: Zi({
				browser_id: this.#e,
				tab_id: this.#a,
				selector: this.#t
			}) })).value;
		}
		async isEnabled() {
			return (await this.#r.send({ command: Fi({
				browser_id: this.#e,
				tab_id: this.#a,
				selector: this.#t
			}) })).value;
		}
		async allTextContents({ timeoutMs: t } = {}) {
			return (await this.#r.send({
				command: Wi({
					browser_id: this.#e,
					tab_id: this.#a,
					selector: this.#t,
					timeout_ms: t
				}),
				timeoutMs: t
			})).values;
		}
		async evaluate(t, a, r) {
			return (await this.#r.send({
				command: Pe({
					browser_id: this.#e,
					script: Vn(t, a),
					selector: this.#t,
					tab_id: this.#a,
					timeout_ms: r?.timeoutMs
				}),
				timeoutMs: r?.timeoutMs
			})).value;
		}
		async evaluateAll(t, a, r) {
			return (await this.#r.send({
				command: Pe({
					browser_id: this.#e,
					script: Vn(t, a, "all"),
					selector: this.#t,
					selector_mode: "all",
					tab_id: this.#a,
					timeout_ms: r?.timeoutMs
				}),
				timeoutMs: r?.timeoutMs
			})).value;
		}
		async actionError(t, a, r) {
			try {
				const s = await this.evaluateAll((c) => ({
					matchCount: c.length,
					visibleCount: c.reduce((o, d) => o + +(d.getClientRects().length > 0), 0),
					matches: c.slice(0, 5).map((o) => ({
						tag: o.tagName.toLowerCase(),
						role: o.getAttribute("role"),
						type: o.getAttribute("type"),
						ariaLabel: o.getAttribute("aria-label"),
						text: o.textContent?.trim().slice(0, 120) ?? "",
						visible: o.getClientRects().length > 0,
						disabled: "disabled" in o && !!o.disabled
					}))
				}), void 0, { timeoutMs: 1e3 });
				return Ne(t, \`\${r}
Locator diagnostics: \${JSON.stringify({
					kind: Jh(t, s),
					action: a,
					locator: this.#t,
					...s,
					truncated: s.matchCount > s.matches.length
				})}\`);
			} catch {
				return Ne(t, r);
			}
		}
		async downloadMedia({ timeoutMs: t = 12e4 } = {}) {
			try {
				const a = await this.#r.send({
					command: yi({
						browser_id: this.#e,
						tab_id: this.#a,
						selector: this.#t,
						timeout_ms: t
					}),
					timeoutMs: t
				});
				return this.#s?.clear(), a.path;
			} catch (a) {
				throw await this.actionError(a, "downloadMedia", \`locator.downloadMedia failed for selector \${this.#t}\`);
			}
		}
		locator(t, a = {}) {
			if (!t) throw new Error("locator.locator requires a selector");
			return new I({
				browserId: this.#e,
				tabId: this.#a,
				selector: \`\${this.#t} >> \${t}\`,
				transport: this.#r,
				collectionReadCache: this.#s,
				collectionReadIndex: this.#i,
				collectionRelativeSelector: this.#s && this.#i !== void 0 ? q(this.#n, t) : void 0
			}).filter(a);
		}
		first() {
			return new I({
				browserId: this.#e,
				tabId: this.#a,
				selector: \`\${this.#t} >> nth=0\`,
				transport: this.#r,
				collectionReadCache: this.#s,
				collectionReadIndex: this.#i,
				collectionRelativeSelector: this.#s && this.#i !== void 0 ? q(this.#n, "nth=0") : void 0
			});
		}
		last() {
			return new I({
				browserId: this.#e,
				tabId: this.#a,
				selector: \`\${this.#t} >> nth=-1\`,
				transport: this.#r,
				collectionReadCache: this.#s,
				collectionReadIndex: this.#i,
				collectionRelativeSelector: this.#s && this.#i !== void 0 ? q(this.#n, "nth=-1") : void 0
			});
		}
		nth(t) {
			if (typeof t != "number") throw new Error("locator.nth requires a numeric index");
			return new I({
				browserId: this.#e,
				tabId: this.#a,
				selector: \`\${this.#t} >> nth=\${t}\`,
				transport: this.#r,
				collectionReadCache: this.#s,
				collectionReadIndex: this.#i,
				collectionRelativeSelector: this.#s && this.#i !== void 0 ? q(this.#n, \`nth=\${t}\`) : void 0
			});
		}
		and(t) {
			if (!(t instanceof I)) throw new Error("locator.and requires a PlaywrightLocator");
			if (t.#e !== this.#e || t.#a !== this.#a) throw new Error("Locators must belong to the same tab");
			return new I({
				browserId: this.#e,
				tabId: this.#a,
				selector: \`\${this.#t} >> internal:and=\${JSON.stringify(t.#t)}\`,
				transport: this.#r
			});
		}
		or(t) {
			if (!(t instanceof I)) throw new Error("locator.or requires a PlaywrightLocator");
			if (t.#e !== this.#e || t.#a !== this.#a) throw new Error("Locators must belong to the same tab");
			return new I({
				browserId: this.#e,
				tabId: this.#a,
				selector: \`\${this.#t} >> internal:or=\${JSON.stringify(t.#t)}\`,
				transport: this.#r
			});
		}
		filter(t = {}) {
			const a = [this.#t];
			if (t.hasText !== void 0 && a.push(\`internal:has-text=\${De(t.hasText, !1)}\`), t.hasNotText !== void 0 && a.push(\`internal:has-not-text=\${De(t.hasNotText, !1)}\`), t.has !== void 0 && (this.assertCompatibleLocator(t.has, "locator.filter has"), a.push(\`internal:has=\${JSON.stringify(t.has.#t)}\`)), t.hasNot !== void 0 && (this.assertCompatibleLocator(t.hasNot, "locator.filter hasNot"), a.push(\`internal:has-not=\${JSON.stringify(t.hasNot.#t)}\`)), t.visible !== void 0) {
				if (typeof t.visible != "boolean") throw new Error("locator.filter visible must be a boolean");
				a.push(\`visible=\${t.visible}\`);
			}
			const r = a.slice(1).join(" >> "), s = this.#s && this.#i !== void 0 && (!r || this.#n !== void 0);
			return new I({
				browserId: this.#e,
				tabId: this.#a,
				selector: a.join(" >> "),
				transport: this.#r,
				collectionReadCache: s ? this.#s : void 0,
				collectionReadIndex: s ? this.#i : void 0,
				collectionRelativeSelector: s ? q(this.#n, r) : void 0
			});
		}
		getByRole(t, a = {}) {
			const r = ut(t, a);
			return new I({
				browserId: this.#e,
				tabId: this.#a,
				selector: \`\${this.#t} >> \${r}\`,
				transport: this.#r,
				collectionReadCache: this.#s,
				collectionReadIndex: this.#i,
				collectionRelativeSelector: this.#s && this.#i !== void 0 ? q(this.#n, r) : void 0
			});
		}
		getByText(t, a = {}) {
			const r = ht(t, a);
			return new I({
				browserId: this.#e,
				tabId: this.#a,
				selector: \`\${this.#t} >> \${r}\`,
				transport: this.#r,
				collectionReadCache: this.#s,
				collectionReadIndex: this.#i,
				collectionRelativeSelector: this.#s && this.#i !== void 0 ? q(this.#n, r) : void 0
			});
		}
		getByLabel(t, a = {}) {
			const r = mt(t, a);
			return new I({
				browserId: this.#e,
				tabId: this.#a,
				selector: \`\${this.#t} >> \${r}\`,
				transport: this.#r,
				collectionReadCache: this.#s,
				collectionReadIndex: this.#i,
				collectionRelativeSelector: this.#s && this.#i !== void 0 ? q(this.#n, r) : void 0
			});
		}
		getByPlaceholder(t, a = {}) {
			const r = pt(t, a);
			return new I({
				browserId: this.#e,
				tabId: this.#a,
				selector: \`\${this.#t} >> \${r}\`,
				transport: this.#r,
				collectionReadCache: this.#s,
				collectionReadIndex: this.#i,
				collectionRelativeSelector: this.#s && this.#i !== void 0 ? q(this.#n, r) : void 0
			});
		}
		getByTestId(t) {
			const a = ft(t);
			return new I({
				browserId: this.#e,
				tabId: this.#a,
				selector: \`\${this.#t} >> \${a}\`,
				transport: this.#r,
				collectionReadCache: this.#s,
				collectionReadIndex: this.#i,
				collectionRelativeSelector: this.#s && this.#i !== void 0 ? q(this.#n, a) : void 0
			});
		}
		async cachedRead(t) {
			return !this.#s || this.#i === void 0 ? null : await this.#s.read({
				index: this.#i,
				relativeSelector: this.#n,
				timeoutMs: t
			});
		}
		assertCompatibleLocator(t, a) {
			if (!(t instanceof I)) throw new Error(\`\${a} requires a PlaywrightLocator\`);
			if (t.#e !== this.#e || t.#a !== this.#a) throw new Error("Locators must belong to the same tab");
		}
	}, Xh = class {
		#e;
		#a;
		#t;
		#r;
		#s = /* @__PURE__ */ new Map();
		constructor({ browserId: e, tabId: t, selector: a, transport: r }) {
			this.#e = e, this.#a = t, this.#t = a, this.#r = r;
		}
		clear() {
			this.#s.clear();
		}
		async read({ index: e, relativeSelector: t, timeoutMs: a }) {
			const r = t ?? "";
			let s = this.#s.get(r);
			return s || (s = this.#i(t, a).catch((c) => {
				throw this.#s.delete(r), c;
			}), this.#s.set(r, s)), (await s)[e] ?? null;
		}
		async #i(e, t) {
			return (await this.#r.send({
				command: sn({
					browser_id: this.#e,
					tab_id: this.#a,
					selector: this.#t,
					relative_selector: e,
					timeout_ms: t
				}),
				timeoutMs: t
			})).values;
		}
	};
	function q(e, t) {
		return e ? t ? \`\${e} >> \${t}\` : e : t || void 0;
	}
	function Gn(e) {
		const t = e;
		return e.unicode || t.unicodeSets ? String(e) : String(e).replace(/(^|[^\\\\])(\\\\\\\\)*(["'\`])/g, "$1$2\\\\\\\\$3").replace(/>>/g, "\\\\>\\\\>");
	}
	function De(e, t) {
		return typeof e != "string" ? Gn(e) : \`\${JSON.stringify(e)}\${t ? "s" : "i"}\`;
	}
	function lt(e, t) {
		return typeof e != "string" ? Gn(e) : \`"\${e.replace(/\\\\/g, "\\\\\\\\").replace(/["]/g, "\\\\\\"")}"\${t ? "s" : "i"}\`;
	}
	function ut(e, t) {
		if (!e) throw new Error("getByRole requires a role");
		const a = [];
		return t.name !== void 0 && a.push(\`[name=\${lt(t.name, !!t.exact)}]\`), \`internal:role=\${e}\${a.join("")}\`;
	}
	function ht(e, t) {
		if (typeof e != "string" && !Ae(e)) throw new Error("getByText requires a string or RegExp");
		return \`internal:text=\${De(e, !!t.exact)}\`;
	}
	function mt(e, t) {
		if (typeof e != "string" && !Ae(e)) throw new Error("getByLabel requires a string or RegExp");
		return \`internal:label=\${De(e, !!t.exact)}\`;
	}
	function pt(e, t) {
		if (typeof e != "string" && !Ae(e)) throw new Error("getByPlaceholder requires a string or RegExp");
		return \`internal:attr=[placeholder=\${lt(e, !!t.exact)}]\`;
	}
	function ft(e) {
		if (!e) throw new Error("getByTestId requires a testId");
		return \`internal:testid=[data-testid=\${lt(e, !0)}]\`;
	}
	var Jn = class Jc {
		#e;
		#a;
		#t;
		#r;
		constructor({ browserId: t, tabId: a, frameSelector: r, transport: s }) {
			this.#e = t, this.#a = a, this.#t = r, this.#r = s;
		}
		locator(t) {
			if (!t) throw new Error("frameLocator.locator requires a selector");
			const a = \`\${this.#t} >> internal:control=enter-frame >> \${t}\`;
			return new ue({
				browserId: this.#e,
				tabId: this.#a,
				selector: a,
				transport: this.#r
			});
		}
		frameLocator(t) {
			if (!t) throw new Error("frameLocator.frameLocator requires a selector");
			return new Jc({
				browserId: this.#e,
				tabId: this.#a,
				frameSelector: \`\${this.#t} >> internal:control=enter-frame >> \${t}\`,
				transport: this.#r
			});
		}
		getByRole(t, a = {}) {
			return this.locator(ut(t, a));
		}
		getByText(t, a = {}) {
			return this.locator(ht(t, a));
		}
		getByLabel(t, a = {}) {
			return this.locator(mt(t, a));
		}
		getByPlaceholder(t, a = {}) {
			return this.locator(pt(t, a));
		}
		getByTestId(t) {
			return this.locator(ft(t));
		}
	}, Kn = class {
		#e;
		#a;
		#t;
		#r;
		constructor({ browserId: e, tabId: t, downloadId: a, transport: r }) {
			this.#e = e, this.#a = t, this.#t = a, this.#r = r;
		}
		async path({ timeoutMs: e } = {}) {
			return (await this.#r.send({
				command: Pn({
					browser_id: this.#e,
					tab_id: this.#a,
					download_id: this.#t,
					timeout_ms: e
				}),
				timeoutMs: e
			})).path ?? null;
		}
	}, Yn = class {
		#e;
		#a;
		#t;
		#r;
		#s;
		constructor({ browserId: e, tabId: t, fileChooserId: a, isMultiple: r, transport: s }) {
			this.#e = e, this.#a = t, this.#t = a, this.#r = r, this.#s = s;
		}
		isMultiple() {
			return this.#r;
		}
		async setFiles(e, { timeoutMs: t } = {}) {
			if (e == null) throw new Error("fileChooser.setFiles requires files");
			const a = Array.isArray(e) ? e : [e];
			if (!a.length) throw new Error("fileChooser.setFiles requires at least one file");
			try {
				await this.#s.send({
					command: Mn({
						browser_id: this.#e,
						tab_id: this.#a,
						file_chooser_id: this.#t,
						files: a,
						timeout_ms: t
					}),
					timeoutMs: t
				});
			} catch (r) {
				throw Ne(r, "fileChooser.setFiles failed");
			}
		}
	}, Xn = class {
		#e;
		#a;
		#t;
		constructor({ browserId: e, tabId: t, transport: a }) {
			this.#e = e, this.#a = t, this.#t = a;
		}
		async goBack() {
			await this.#t.send({ command: it({
				browser_id: this.#e,
				tab_id: this.#a
			}) });
		}
		async goForward() {
			await this.#t.send({ command: nt({
				browser_id: this.#e,
				tab_id: this.#a
			}) });
		}
		async evaluate(e, t, a) {
			return (await this.#t.send({
				command: Pe({
					browser_id: this.#e,
					script: Kh(e, t),
					tab_id: this.#a,
					timeout_ms: a?.timeoutMs
				}),
				timeoutMs: a?.timeoutMs
			})).value;
		}
		locator(e) {
			if (!e) throw new Error("playwright.locator requires a selector");
			return new ue({
				browserId: this.#e,
				tabId: this.#a,
				selector: e,
				transport: this.#t
			});
		}
		getByRole(e, t = {}) {
			return this.locator(ut(e, t));
		}
		getByText(e, t = {}) {
			return this.locator(ht(e, t));
		}
		getByLabel(e, t = {}) {
			return this.locator(mt(e, t));
		}
		getByPlaceholder(e, t = {}) {
			return this.locator(pt(e, t));
		}
		getByTestId(e) {
			return this.locator(ft(e));
		}
		frameLocator(e) {
			if (!e) throw new Error("playwright.frameLocator requires a selector");
			return new Jn({
				browserId: this.#e,
				tabId: this.#a,
				frameSelector: e,
				transport: this.#t
			});
		}
		async waitForURL(e, t = {}) {
			if (!e) throw new Error("playwright.waitForURL requires a url");
			await this.#t.send({
				command: cn({
					browser_id: this.#e,
					tab_id: this.#a,
					url: e,
					wait_until: t.waitUntil,
					timeout_ms: t.timeoutMs
				}),
				timeoutMs: t.timeoutMs
			});
		}
		async waitForLoadState(e = {}) {
			await this.#t.send({
				command: hn({
					browser_id: this.#e,
					tab_id: this.#a,
					state: e.state,
					timeout_ms: e.timeoutMs
				}),
				timeoutMs: e.timeoutMs
			});
		}
		async waitForTimeout(e) {
			if (!Number.isInteger(e) || e < 0) throw new Error("playwright.waitForTimeout requires a non-negative integer");
			await this.#t.send({
				command: fn({
					browser_id: this.#e,
					tab_id: this.#a,
					timeout_ms: e
				}),
				timeoutMs: e + 2e3
			});
		}
		waitForEvent(e, t = {}) {
			const a = this.#r(e, t);
			return a.catch(() => {}), a;
		}
		async #r(e, t) {
			if (e === "download") {
				const a = t.timeoutMs ?? 12e4, r = await this.#t.send({
					command: Rn({
						browser_id: this.#e,
						tab_id: this.#a,
						timeout_ms: a
					}),
					timeoutMs: a
				});
				return new Kn({
					browserId: this.#e,
					tabId: this.#a,
					downloadId: r.download_id,
					transport: this.#t
				});
			}
			if (e === "filechooser") {
				const a = await this.#t.send({
					command: Nn({
						browser_id: this.#e,
						tab_id: this.#a,
						timeout_ms: t.timeoutMs
					}),
					timeoutMs: t.timeoutMs
				});
				return new Yn({
					browserId: this.#e,
					tabId: this.#a,
					fileChooserId: a.file_chooser_id,
					isMultiple: a.is_multiple,
					transport: this.#t
				});
			}
			throw new Error("playwright.waitForEvent only supports 'download' and 'filechooser'");
		}
		async expectNavigation(e, t = {}) {
			const a = t.url ? this.waitForURL(t.url, {
				timeoutMs: t.timeoutMs,
				waitUntil: t.waitUntil
			}) : this.waitForLoadState({
				timeoutMs: t.timeoutMs,
				state: t.waitUntil
			}), r = Promise.resolve().then(e), [s] = await Promise.all([r, a]);
			return s;
		}
		async elementInfo(e) {
			const t = Wn("playwright.elementInfo", e);
			return await this.#t.send({ command: vn({
				browser_id: this.#e,
				tab_id: this.#a,
				x: t.x,
				y: t.y,
				include_non_interactable: e.includeNonInteractable
			}) });
		}
		async elementScreenshot(e) {
			const t = Wn("playwright.elementScreenshot", e);
			return Ee((await this.#t.send({ command: Tn({
				browser_id: this.#e,
				tab_id: this.#a,
				x: t.x,
				y: t.y,
				include_non_interactable: e.includeNonInteractable
			}) })).data);
		}
		async domSnapshot() {
			return (await this.#t.send({ command: bn({
				browser_id: this.#e,
				tab_id: this.#a
			}) })).dom_snapshot;
		}
	}, Qn = class {
		#e;
		#a;
		#t;
		constructor({ browserId: e, tabId: t, transport: a }) {
			this.#e = e, this.#a = t, this.#t = a;
		}
		async get_visible_dom() {
			return this.#t.send({ command: Xs({
				browser_id: this.#e,
				tab_id: this.#a
			}) });
		}
		async click(e) {
			const t = yt(e?.node_id, "dom_cua.click");
			await this.#t.send({ command: Fs({
				browser_id: this.#e,
				tab_id: this.#a,
				node_id: t
			}) });
		}
		async double_click(e) {
			const t = yt(e?.node_id, "dom_cua.double_click");
			await this.#t.send({ command: Js({
				browser_id: this.#e,
				tab_id: this.#a,
				node_id: t
			}) });
		}
		async scroll({ node_id: e, x: t, y: a }) {
			if (typeof t != "number" || typeof a != "number") throw new Error("dom_cua.scroll requires x and y numbers");
			const r = e === void 0 ? void 0 : eo(e, "dom_cua.scroll");
			await this.#t.send({ command: si({
				browser_id: this.#e,
				tab_id: this.#a,
				node_id: r,
				scroll_x: t,
				scroll_y: a
			}) });
		}
		async type({ text: e }) {
			if (typeof e != "string") throw new Error("dom_cua.type requires text");
			await this.#t.send({ command: oi({
				browser_id: this.#e,
				tab_id: this.#a,
				text: e
			}) });
		}
		async keypress(e) {
			if (!Array.isArray(e?.keys) || e.keys.length === 0) throw new Error("dom_cua.keypress requires a non-empty keys array");
			await this.#t.send({ command: ti({
				browser_id: this.#e,
				tab_id: this.#a,
				keys: e.keys
			}) });
		}
		async downloadMedia(e) {
			const t = yt(e?.node_id, "dom_cua.downloadMedia");
			await this.#t.send({
				command: Ws({
					browser_id: this.#e,
					tab_id: this.#a,
					node_id: t,
					timeout_ms: e.timeoutMs
				}),
				timeoutMs: e.timeoutMs
			});
		}
	};
	function yt(e, t) {
		if (e === void 0) throw new Error(\`\${t} requires a node_id\`);
		return eo(e, t);
	}
	function eo(e, t) {
		const a = te.safeParse(e);
		if (!a.success) throw new Error(\`\${t} \${a.error.issues[0]?.message ?? "has an invalid node_id"}\`);
		return a.data;
	}
	var to = class {
		#e;
		#a;
		#t;
		constructor({ browserId: e, tabId: t, transport: a }) {
			this.#e = e, this.#a = t, this.#t = a;
		}
		async click(e) {
			if (typeof e?.x != "number" || typeof e?.y != "number") throw new Error("cua.click requires x and y");
			await this.#t.send({ command: gs({
				browser_id: this.#e,
				tab_id: this.#a,
				x: e.x,
				y: e.y,
				button: e.button,
				keys: e.keypress
			}) });
		}
		async scroll(e) {
			if (typeof e?.x != "number" || typeof e?.y != "number" || typeof e?.scrollX != "number" || typeof e?.scrollY != "number") throw new Error("cua.scroll requires x, y, scrollX, and scrollY");
			await this.#t.send({ command: Bs({
				browser_id: this.#e,
				tab_id: this.#a,
				x: e.x,
				y: e.y,
				scroll_x: e.scrollX,
				scroll_y: e.scrollY,
				keys: e.keypress
			}) });
		}
		async double_click(e) {
			if (typeof e?.x != "number" || typeof e?.y != "number") throw new Error("cua.double_click requires x and y");
			await this.#t.send({ command: $s({
				browser_id: this.#e,
				tab_id: this.#a,
				x: e.x,
				y: e.y,
				keys: e.keypress
			}) });
		}
		async type(e) {
			if (typeof e?.text != "string") throw new Error("cua.type requires text");
			await this.#t.send({ command: Zs({
				browser_id: this.#e,
				tab_id: this.#a,
				text: e.text
			}) });
		}
		async keypress(e) {
			if (!Array.isArray(e?.keys) || e.keys.length === 0) throw new Error("cua.keypress requires a non-empty keys array");
			await this.#t.send({ command: Cs({
				browser_id: this.#e,
				tab_id: this.#a,
				keys: e.keys
			}) });
		}
		async drag(e) {
			if (!Array.isArray(e?.path) || e.path.length === 0 || e.path.some((t) => typeof t?.x != "number" || typeof t?.y != "number")) throw new Error("cua.drag requires a non-empty path of {x, y} points");
			await this.#t.send({ command: Es({
				browser_id: this.#e,
				tab_id: this.#a,
				path: e.path,
				keys: e.keys
			}) });
		}
		async move(e) {
			if (typeof e?.x != "number" || typeof e?.y != "number") throw new Error("cua.move requires x and y");
			await this.#t.send({ command: Ds({
				browser_id: this.#e,
				tab_id: this.#a,
				x: e.x,
				y: e.y,
				keys: e.keys
			}) });
		}
		async downloadMedia(e) {
			if (typeof e?.x != "number" || typeof e?.y != "number") throw new Error("cua.downloadMedia requires x and y");
			await this.#t.send({
				command: Ss({
					browser_id: this.#e,
					tab_id: this.#a,
					x: e.x,
					y: e.y,
					timeout_ms: e.timeoutMs
				}),
				timeoutMs: e.timeoutMs
			});
		}
	}, ao = class {
		#e;
		#a;
		#t;
		constructor({ browserId: e, tabId: t, transport: a }) {
			this.#e = e, this.#a = t, this.#t = a;
		}
		async get(e = "state", t) {
			const a = await this.#r(e, t);
			switch (a.mode) {
				case "state": return a.state;
				case "screenshot": return a.screenshot;
				case "both": return a.screenshot === void 0 ? { state: a.state } : {
					state: a.state,
					screenshot: a.screenshot
				};
			}
		}
		async write(e = "state", t) {
			const a = await this.#r(e, t);
			a.mode !== "screenshot" && await this.#t.display(a.state), a.mode !== "state" && a.screenshot !== void 0 && await this.#t.display(a.screenshot);
		}
		async click(e, t) {
			await this.#s({
				kind: "click",
				target: e,
				...t?.mouseButton === void 0 ? {} : { mouse_button: t.mouseButton },
				...t?.clickCount === void 0 ? {} : { click_count: t.clickCount },
				...t?.key === void 0 ? {} : { key: t.key },
				...t?.duration === void 0 ? {} : { duration: t.duration }
			});
		}
		async drag(e, t) {
			await this.#s({
				kind: "drag",
				from: e,
				to: t
			});
		}
		async paste(e, t, a) {
			await this.#s({
				kind: "paste",
				element_index: e,
				text: t,
				format: a?.format
			});
		}
		async performSecondaryAction(e, t) {
			await this.#s({
				kind: "perform_secondary_action",
				element_index: e,
				action: t
			});
		}
		async pressKey(e, t, a) {
			await this.#s({
				kind: "press_key",
				element_index: e,
				key: t,
				...a?.duration === void 0 ? {} : { duration: a.duration }
			});
		}
		async scroll(e, t, a) {
			await this.#s({
				kind: "scroll",
				target: e,
				direction: t,
				...a === void 0 ? {} : { pages: a }
			});
		}
		async selectText(e, t, a) {
			await this.#s({
				kind: "select_text",
				element_index: e,
				text: t,
				...a?.prefix === void 0 ? {} : { prefix: a.prefix },
				...a?.suffix === void 0 ? {} : { suffix: a.suffix },
				...a?.selectionType === void 0 ? {} : { selection_type: a.selectionType }
			});
		}
		async setValue(e, t) {
			await this.#s({
				kind: "set_value",
				element_index: e,
				value: t
			});
		}
		async typeText(e, t) {
			await this.#s({
				kind: "type_text",
				element_index: e,
				text: t
			});
		}
		async #r(e, t) {
			const a = await this.#t.send({ command: ds({
				browser_id: this.#e,
				tab_id: this.#a,
				content: e === "state" ? "axState" : e === "screenshot" ? "screenshot" : "axStateAndScreenshot",
				...t?.disableDiffing === void 0 ? {} : { disable_diffing: t.disableDiffing }
			}) });
			if (e === "state") {
				if (typeof a.state != "string") throw new Error("ax capture returned no accessibility state");
				return {
					mode: e,
					state: a.state
				};
			}
			if (typeof a.data != "string") {
				if (e === "both" && typeof a.screenshot_unavailable == "string" && typeof a.state == "string") return {
					mode: e,
					state: a.state
				};
				throw new Error("ax capture returned no screenshot data");
			}
			const r = Ee(a.data);
			if (e === "screenshot") return {
				mode: e,
				screenshot: r
			};
			if (typeof a.state != "string") throw new Error("ax capture returned no accessibility state");
			return {
				mode: e,
				state: a.state,
				screenshot: r
			};
		}
		async #s(e) {
			await this.#t.send({ command: ps({
				browser_id: this.#e,
				tab_id: this.#a,
				action: e
			}) });
		}
	}, ro = class {
		#e;
		#a;
		#t;
		constructor({ browserId: e, tabId: t, transport: a }) {
			this.#e = e, this.#a = t, this.#t = a;
		}
		async export() {
			return (await this.#t.send({ command: jr({
				browser_id: this.#e,
				tab_id: this.#a
			}) })).path;
		}
		async exportGsuite(e) {
			return (await this.#t.send({ command: Ur({
				browser_id: this.#e,
				tab_id: this.#a,
				format: e
			}) })).path;
		}
		async exportYouTubeTranscript() {
			return (await this.#t.send({ command: Hr({
				browser_id: this.#e,
				tab_id: this.#a
			}) })).path;
		}
	}, so = class {
		#e;
		#a;
		#t;
		constructor({ browserId: e, tabId: t, transport: a }) {
			this.#e = e, this.#a = t, this.#t = a;
		}
		async readText() {
			return (await this.#t.send({ command: Gr({
				browser_id: this.#e,
				tab_id: this.#a
			}) })).text;
		}
		async writeText(e) {
			if (e == null) throw new Error("tab.clipboard.writeText requires text");
			await this.#t.send({ command: Yr({
				browser_id: this.#e,
				tab_id: this.#a,
				text: e
			}) });
		}
		async read() {
			return (await this.#t.send({ command: es({
				browser_id: this.#e,
				tab_id: this.#a
			}) })).items.map((e) => ({
				entries: e.entries.map((t) => ({
					mimeType: t.mime_type,
					text: t.text,
					base64: t.base64
				})),
				presentationStyle: e.presentation_style
			}));
		}
		async write(e) {
			if (!Array.isArray(e) || e.length === 0) throw new Error("tab.clipboard.write requires at least one clipboard item");
			await this.#t.send({ command: rs({
				browser_id: this.#e,
				tab_id: this.#a,
				items: e.map((t) => ({
					presentation_style: t.presentationStyle,
					entries: t.entries.map((a) => ({
						mime_type: a.mimeType,
						text: a.text,
						base64: a.base64
					}))
				}))
			}) });
		}
	}, io = class {
		#e;
		#a;
		#t;
		constructor({ browserId: e, tabId: t, transport: a }) {
			this.#e = e, this.#a = t, this.#t = a;
		}
		async logs(e = {}) {
			const t = Qh(e.filter), a = em(e.levels), r = tm(e.limit), s = {
				browser_id: this.#e,
				tab_id: this.#a,
				...t !== void 0 ? { filter: t } : {},
				...a !== void 0 ? { levels: a } : {},
				...r !== void 0 ? { limit: r } : {}
			};
			return (await this.#t.send({ command: Cr(s) })).logs;
		}
	};
	const Qh = (e) => {
		if (e != null) {
			if (typeof e != "string") throw new Error("tab.dev.logs received an invalid filter");
			return e;
		}
	}, em = (e) => {
		if (e != null) {
			if (!Array.isArray(e) || e.length === 0) throw new Error("tab.dev.logs received invalid levels");
			return e.map((t) => {
				if (t === "warning") return "warn";
				if (Ce.options.includes(t)) return t;
				throw new Error(\`tab.dev.logs received invalid level "\${String(t)}"\`);
			});
		}
	}, tm = (e) => {
		if (e != null) {
			if (!Number.isInteger(e) || e <= 0) throw new Error("tab.dev.logs received an invalid limit");
			return e;
		}
	};
	var _t = class {
		constructor(e, t, a, r) {
			this.transport = e, this.browserId = t, this.documentationApi = a, this.info = r;
		}
		get id() {
			return this.info.id;
		}
		async documentation() {
			return await this.documentationApi.get(\`capabilities/browser/\${this.id}\`);
		}
	}, ae = class {
		constructor(e, t, a, r, s) {
			this.transport = e, this.browserId = t, this.tabId = a, this.documentationApi = r, this.info = s;
		}
		get id() {
			return this.info.id;
		}
		async documentation() {
			return await this.documentationApi.get(\`capabilities/tab/\${this.id}\`);
		}
	};
	function bt({ capability: e, info: t, internalOnly: a }) {
		return {
			capability: e,
			create: (r) => new e({
				...r,
				info: t
			}),
			id: t.id,
			info: t,
			...a == null ? {} : { internalOnly: a }
		};
	}
	function re({ capability: e, info: t, internalOnly: a }) {
		return {
			capability: e,
			create: (r) => new e({
				...r,
				info: t
			}),
			id: t.id,
			info: t,
			...a == null ? {} : { internalOnly: a }
		};
	}
	var no = class {
		#e;
		constructor(e) {
			this.#e = e;
		}
		async get(e) {
			const t = this.#e[e];
			if (!t) throw new Error(\`Capability is not available: \${e}\`);
			return t;
		}
		async list() {
			return Object.values(this.#e).map((e) => e.info);
		}
	};
	const rm = n({ reason: S([
		"captcha_failed",
		"access_denied",
		"challenge_loop",
		"sign_in_blocked",
		"add_to_cart_blocked",
		"purchase_blocked",
		"unexpected_bot_error"
	]) });
	var sm = l({
		PayloadSchema: () => gt,
		ResultSchema: () => co,
		commandType: () => oo,
		create: () => lo
	});
	const oo = "tab_bot_detection_report", gt = rm.extend({
		browser_id: i(),
		tab_id: i()
	}), co = n({
		status: C("reported"),
		hostname: i().nullable()
	});
	function lo(e) {
		return new h(oo, gt, e);
	}
	var im = l({ Report: () => sm });
	const nm = {
		id: "botDetection",
		description: "Use when the current tab is blocked by bot detection, a failed CAPTCHA, a hard access denial, or a repeated challenge/login loop."
	};
	var uo = class extends ae {
		constructor({ browserId: e, documentation: t, info: a, tabId: r, transport: s }) {
			super(s, e, r, t, a);
		}
		async report(e) {
			const t = gt.parse({
				...e,
				browser_id: this.browserId,
				tab_id: this.tabId
			}), a = await this.transport.send({ command: lo(t) });
			return co.parse(a);
		}
	};
	const ho = re({
		capability: uo,
		info: nm
	}), mo = n({
		session_id: i().min(1).optional(),
		target_id: i().min(1).optional()
	}).refine((e) => e.session_id == null != (e.target_id == null), "CDP target must provide exactly one of session_id or target_id."), om = n({
		extensionId: i().optional(),
		sessionId: i().optional(),
		tabId: u().int().optional(),
		targetId: i().optional()
	}), cm = n({
		sequence: u().int().positive(),
		source: om,
		method: i().min(1),
		params: de(i(), Z()).optional()
	});
	var dm = l({
		PayloadSchema: () => fo,
		ResultSchema: () => yo,
		commandType: () => po,
		create: () => _o
	});
	const po = "tab_cdp_call", fo = n({
		browser_id: i(),
		tab_id: i(),
		method: i().min(1),
		params: de(i(), Z()).optional(),
		target: mo.optional(),
		timeout_ms: u().int().positive().optional()
	}), yo = Z();
	function _o(e) {
		return new h(po, fo, e);
	}
	var lm = l({
		PayloadSchema: () => go,
		ResultSchema: () => wo,
		commandType: () => bo,
		create: () => vo
	});
	const bo = "tab_cdp_events", go = n({
		after_sequence: u().int().nonnegative().optional(),
		browser_id: i(),
		limit: u().int().positive().max(1e3).optional(),
		methods: g(i().min(1)).min(1).optional(),
		tab_id: i(),
		target: mo.optional(),
		timeout_ms: u().int().nonnegative().optional()
	}), wo = n({
		cursor: u().int().nonnegative(),
		events: g(cm),
		hasMore: k(),
		truncated: k()
	});
	function vo(e) {
		return new h(bo, go, e);
	}
	var um = l({
		Call: () => dm,
		Events: () => lm
	});
	const hm = {
		id: "cdp",
		description: "Send raw Chrome DevTools Protocol commands and read debugger events through a supported tab for developer use cases."
	}, mm = 2e3;
	var So = class extends ae {
		constructor({ browserId: e, documentation: t, info: a, tabId: r, transport: s }) {
			super(s, e, r, t, a);
		}
		async send(e, t, a) {
			const r = await this.transport.send({
				command: _o({
					browser_id: this.browserId,
					method: e,
					params: t,
					tab_id: this.tabId,
					target: To(a?.target),
					timeout_ms: a?.timeoutMs
				}),
				timeoutMs: a?.timeoutMs
			});
			return yo.parse(r);
		}
		async readEvents(e) {
			const t = await this.transport.send({
				command: vo({
					after_sequence: e?.afterSequence,
					browser_id: this.browserId,
					limit: e?.limit,
					methods: e?.methods,
					tab_id: this.tabId,
					target: To(e?.target),
					timeout_ms: e?.timeoutMs
				}),
				timeoutMs: pm(e?.timeoutMs)
			});
			return wo.parse(t);
		}
	};
	const xo = re({
		capability: So,
		info: hm
	});
	function To(e) {
		if (e != null) return {
			session_id: e.sessionId,
			target_id: e.targetId
		};
	}
	function pm(e) {
		return e != null && e > 0 ? e + mm : void 0;
	}
	const $o = /^[A-Za-z0-9_-]{1,48}$/, ko = new Set([
		"__proto__",
		"constructor",
		"prototype"
	]), fm = n({
		id: i().regex($o).refine((e) => !ko.has(e)),
		label: i(),
		type: i().min(1),
		autocomplete: i().nullable().optional(),
		required: k(),
		selector: i()
	}), ym = n({
		selector: i(),
		action: S(["click", "press_enter"])
	}), _m = n({
		id: i().regex($o).refine((e) => !ko.has(e)),
		label: i().trim().min(1).max(120),
		selector: i().optional(),
		field_ids: g(i()).max(6).optional()
	}).refine((e) => e.selector != null != (e.field_ids?.length ?? 0) > 0, "browser auth options require either a selector or credential fields"), bm = n({
		origin: i(),
		account_identifier: i().min(1).max(320).refine((e) => e.trim().length > 0).optional(),
		fields: g(fm).max(6),
		options: g(_m).min(2).max(10).optional(),
		qr_code: C(!0).optional(),
		submit: ym.optional(),
		otpHandle: i().min(1).max(16384).optional()
	}).refine((e) => e.account_identifier == null || e.fields.length === 1 && e.fields[0]?.type === "password" && e.options == null && e.qr_code == null && e.otpHandle == null, "account_identifier is only supported for a password-only sign-in").refine((e) => e.fields.length > 0 || e.options != null || e.qr_code === !0, "browser auth requests require credential fields, sign-in options, or a QR code");
	var gm = l({
		PayloadSchema: () => Co,
		ResultSchema: () => Eo,
		SubmissionAttemptEventSchema: () => Io,
		SubmissionDiagnosticsSchema: () => Po,
		SubmissionRejectionSchema: () => wt,
		commandType: () => Ro,
		create: () => Ao
	});
	const Ro = "tab_browser_auth_handoff", Co = bm.and(n({
		browser_id: i(),
		tab_id: i()
	})), wt = n({
		outcome: S(["origin_changed", "submission_failed"]),
		reason: S([
			"request_url_invalid",
			"request_scheme",
			"request_origin",
			"paused_iframe",
			"iframe_origin",
			"child_frame_origin",
			"other"
		]),
		request_class: S([
			"main_document",
			"background",
			"other"
		]),
		submission_phase: S([
			"pending",
			"settled",
			"not_applicable"
		])
	}), Io = n({
		sequence: u().int().min(0).max(65535),
		kind: S([
			"prompt_completed",
			"field_fill_started",
			"field_fill_completed",
			"field_fill_uncertain",
			"url_changed",
			"document_changed",
			"frame_changed",
			"reinspection_started",
			"reinspection_succeeded",
			"reinspection_failed",
			"site_request",
			"site_navigation",
			"site_submission_verified",
			"submit_started",
			"attempt_completed"
		]),
		phase: S([
			"prompt",
			"filling",
			"before_submit",
			"submitting",
			"settled"
		]),
		field_index: u().int().min(0).max(63).optional(),
		reinspection: u().int().min(1).max(3).optional(),
		outcome: S([
			"submitted",
			"unavailable",
			"origin_changed",
			"page_changed",
			"locator_invalid",
			"submission_failed",
			"prompt_changed"
		]).optional(),
		request_class: S(["main_document", "background"]).optional(),
		method: S([
			"get",
			"post",
			"other"
		]).optional(),
		origin_relation: S([
			"same",
			"cross",
			"other"
		]).optional()
	}), Po = n({
		schema_version: C(1),
		request_id: i().uuid(),
		submission_mode: S(["ordinary", "private"]),
		first_rejection: wt.optional(),
		terminal_rejection: wt.optional(),
		attempt_events: g(Io).max(64).optional(),
		dropped_attempt_events: u().int().min(0).max(65535).optional()
	}), Eo = n({
		status: S([
			"submitted",
			"declined",
			"cancelled",
			"unavailable",
			"expired",
			"origin_changed",
			"page_changed",
			"locator_invalid",
			"submission_failed"
		]),
		locator_error: n({
			field_id: i(),
			reason: C("not_user_visible")
		}).optional(),
		selected_option: i().optional(),
		reason: C("user_took_over").optional(),
		submission_diagnostics: Po.optional()
	});
	function Ao(e) {
		return new h(Ro, Co, e);
	}
	var wm = l({ Handoff: () => gm });
	const vm = {
		id: "browserAuth",
		description: "MUST read its documentation before the first interaction with any authentication or sign-in flow required for the task. Covers method selection, secure user-provided credentials, and post-authentication verification and recovery."
	};
	var No = class extends ae {
		constructor({ browserId: e, documentation: t, info: a, tabId: r, transport: s }) {
			super(s, e, r, t, a);
		}
		async request(e) {
			const a = Ao({
				...Sm(e, {
					browserId: this.browserId,
					tabId: this.tabId
				}),
				browser_id: this.browserId,
				tab_id: this.tabId
			});
			a.parse();
			const r = await this.transport.send({ command: a });
			return Eo.parse(r);
		}
	};
	const Do = re({
		capability: No,
		info: vm
	});
	function Sm(e, t) {
		const a = {
			origin: e.origin,
			account_identifier: e.account_identifier,
			otpHandle: e.otpHandle,
			fields: e.fields.map((r) => {
				const s = {
					id: r.id,
					label: r.label,
					type: r.type,
					required: r.required,
					selector: vt(r.selector, t)
				};
				return r.autocomplete !== void 0 && (s.autocomplete = r.autocomplete), s;
			}),
			...e.qr_code === !0 ? { qr_code: !0 } : {}
		};
		return e.options != null && (a.options = e.options.map((r) => ({
			id: r.id,
			label: r.label,
			...r.field_ids == null ? {} : { field_ids: r.field_ids },
			...r.selector == null ? {} : { selector: vt(r.selector, t) }
		}))), e.submit != null && (a.submit = {
			action: e.submit.action,
			selector: vt(e.submit.selector, t)
		}), a;
	}
	function vt(e, t) {
		if (typeof e == "string") return e;
		if (!(e instanceof ue)) throw new Error("browserAuth selector must be a string or PlaywrightLocator");
		const a = ue.browserAuthSelector(e);
		if (a.browserId !== t.browserId || a.tabId !== t.tabId) throw new Error("browserAuth selector locator must belong to this tab");
		return a.selector;
	}
	const Oo = S([
		"font",
		"image",
		"script",
		"stylesheet",
		"video",
		"other"
	]), Mo = S([
		"font",
		"image",
		"stylesheet",
		"video"
	]), xm = n({
		kind: S([
			"attribute",
			"computedStyle",
			"resource"
		]),
		nodeId: u().int().positive().optional(),
		property: i().optional()
	}), Tm = n({
		id: i(),
		kind: Oo,
		name: i(),
		sources: g(xm),
		url: i()
	}), $m = n({
		id: i(),
		markup: i(),
		name: i()
	}), km = n({
		assets: g(Tm),
		id: i(),
		inlineSvgs: g($m),
		pageUrl: i().nullable(),
		summary: n({
			byKind: de(Oo, u().int().nonnegative()),
			inlineSvgCount: u().int().nonnegative(),
			totalCount: u().int().nonnegative()
		})
	}), Rm = n({
		assetIds: g(i()).optional(),
		inventoryId: i(),
		kinds: g(Mo).optional()
	}), Cm = n({
		contentType: i().nullable(),
		id: i(),
		kind: Mo,
		name: i(),
		path: i(),
		url: i()
	}), Im = n({
		contentType: i().nullable(),
		id: i(),
		name: i(),
		reason: i(),
		url: i()
	}), Pm = n({
		assets: g(Cm),
		directoryPath: i(),
		failures: g(Im),
		manifestPath: i(),
		summary: n({
			downloadedCount: u().int().nonnegative(),
			elapsedMs: u().nonnegative(),
			failedCount: u().int().nonnegative(),
			requestedCount: u().int().nonnegative()
		})
	});
	var Em = l({
		PayloadSchema: () => jo,
		ResultSchema: () => Lo,
		commandType: () => Bo,
		create: () => Zo
	});
	const Bo = "tab_page_assets_bundle", jo = Rm.extend({
		browser_id: i(),
		tab_id: i()
	}), Lo = Pm;
	function Zo(e) {
		return new h(Bo, jo, e);
	}
	var Am = l({
		PayloadSchema: () => Uo,
		ResultSchema: () => Fo,
		commandType: () => qo,
		create: () => Vo
	});
	const qo = "tab_page_assets_list", Uo = n({
		browser_id: i(),
		tab_id: i()
	}), Fo = km;
	function Vo(e) {
		return new h(qo, Uo, e);
	}
	var Nm = l({
		Bundle: () => Em,
		List: () => Am
	});
	const Dm = {
		id: "pageAssets",
		description: "List assets already observed in the current page state and bundle selected assets into a temporary local artifact."
	};
	var Ho = class extends ae {
		constructor({ browserId: e, documentation: t, info: a, tabId: r, transport: s }) {
			super(s, e, r, t, a);
		}
		async list() {
			const e = await this.transport.send({ command: Vo({
				browser_id: this.browserId,
				tab_id: this.tabId
			}) });
			return Fo.parse(e);
		}
		async bundle(e) {
			const t = await this.transport.send({ command: Zo({
				...e,
				browser_id: this.browserId,
				tab_id: this.tabId
			}) });
			return Lo.parse(t);
		}
	};
	const Wo = re({
		capability: Ho,
		info: Dm
	}), zo = {
		id: "webmcp",
		description: "Fetch page-defined WebMCP tools bound to the current document, then call them through the returned object."
	};
	function Go(e) {
		return {
			name: e.name,
			call_name: e.call_name,
			...e.title == null ? {} : { title: e.title },
			...e.description == null ? {} : { description: e.description },
			inputSchema: e.input_schema,
			...e.annotations == null ? {} : { annotations: e.annotations },
			...e.origin == null ? {} : { origin: e.origin },
			...e.pageUrl == null ? {} : { pageUrl: e.pageUrl }
		};
	}
	var Jo = class extends ae {
		constructor({ browserId: e, documentation: t, info: a, tabId: r, transport: s }) {
			super(s, e, r, t, a);
		}
		async fetchTools() {
			const e = Ya.parse(await this.transport.send({ command: Qa({
				browser_id: this.browserId,
				tab_id: this.tabId
			}) })), t = e.tools.map(Go), a = new Map(e.tools.map((s) => [s.call_name, s])), r = Om(t);
			return Object.freeze({
				description: () => r,
				call: async (s, c, o) => {
					const d = s.trim(), p = a.get(d);
					if (p == null) throw new Error(\`WebMCP tool \${JSON.stringify(d)} is not available in this snapshot. Call fetchTools() again.\`);
					return (await this.transport.send({ command: ar({
						browser_id: this.browserId,
						tab_id: this.tabId,
						tool_name: p.name,
						tool_description: p.description,
						tool_title: p.title,
						registration_id: p.registration_id,
						input: c,
						...o?.timeoutMs == null ? {} : { timeout_ms: o.timeoutMs }
					}) })).result;
				}
			});
		}
	};
	function Om(e) {
		return e.length === 0 ? "No WebMCP tools are available in this document." : [
			"WebMCP tools available in this document:",
			JSON.stringify(e.map(({ call_name: t, ...a }) => ({
				...a,
				name: t
			})), null, 2),
			"Call tools.call(name, input) to invoke one."
		].join(\`
\`);
	}
	const Ko = re({
		capability: Jo,
		info: zo
	}), Mm = {
		id: "siwc",
		description: "Detect possible Sign in with ChatGPT controls and authenticate the user. Read its documentation before authenticating with the current account."
	};
	var Yo = class extends ae {
		constructor({ browserId: e, documentation: t, info: a, tabId: r, transport: s }) {
			super(s, e, r, t, a);
		}
		async detect() {
			return await this.transport.send({ command: Da({
				browser_id: this.browserId,
				tab_id: this.tabId
			}) });
		}
		async authenticate(e = {}) {
			const t = Ea({
				...e,
				browser_id: this.browserId,
				tab_id: this.tabId
			});
			return t.parse(), await this.transport.send({ command: t });
		}
	};
	const Xo = re({
		capability: Yo,
		info: Mm
	}), Qo = [
		xo,
		ho,
		Do,
		Wo,
		Ko,
		Xo
	], Bm = Lm(Qo), jm = ({ browserId: e, capabilities: t, documentation: a, tabId: r, transport: s }) => {
		const c = {};
		for (const o of t ?? []) {
			const d = Bm.get(o.id);
			d != null && Object.assign(c, { [o.id]: d({
				browserId: e,
				documentation: a,
				info: o,
				tabId: r,
				transport: s
			}) });
		}
		return new no(c);
	};
	function Lm(e) {
		return new Map(e.map((t) => [t.id, t.create]));
	}
	var Oe = class {
		constructor({ browserId: e, dialogId: t, tabId: a, transport: r, type: s }) {
			this.browserId = e, this.dialogId = t, this.tabId = a, this.transport = r, this.type = s;
		}
		async dismiss() {
			await this.transport.send({ command: Ie({
				action: "dismiss",
				browser_id: this.browserId,
				dialog_id: this.dialogId,
				tab_id: this.tabId
			}) });
		}
	}, ec = class extends Oe {
		constructor(e) {
			super({
				...e,
				type: "alert"
			});
		}
	}, tc = class extends Oe {
		constructor(e) {
			super({
				...e,
				type: "beforeunload"
			});
		}
	}, ac = class extends Oe {
		constructor(e) {
			super({
				...e,
				type: "confirm"
			});
		}
		async accept() {
			await this.transport.send({ command: Ie({
				action: "accept",
				browser_id: this.browserId,
				dialog_id: this.dialogId,
				tab_id: this.tabId
			}) });
		}
	}, rc = class extends Oe {
		constructor(e) {
			super({
				...e,
				type: "prompt"
			});
		}
		async accept(e) {
			if (typeof e != "string") throw new Error("prompt.accept requires text");
			await this.transport.send({ command: Ie({
				action: "accept",
				browser_id: this.browserId,
				dialog_id: this.dialogId,
				prompt_text: e,
				tab_id: this.tabId
			}) });
		}
	}, St = class {
		#e;
		#a;
		constructor({ browserId: e, capabilities: t, documentation: a, transport: r, tabPayload: s = {} }) {
			if (!s.id) throw new Error("Tab requires an id");
			this.#e = e, this.#a = r, this.id = s.id, this.playwright = new Xn({
				browserId: e,
				tabId: this.id,
				transport: this.#a
			}), this.dom_cua = new Qn({
				browserId: e,
				tabId: this.id,
				transport: this.#a
			}), this.cua = new to({
				browserId: e,
				tabId: this.id,
				transport: this.#a
			}), this.ax = new ao({
				browserId: e,
				tabId: this.id,
				transport: this.#a
			}), this.content = new ro({
				browserId: e,
				tabId: this.id,
				transport: this.#a
			}), this.clipboard = new so({
				browserId: e,
				tabId: this.id,
				transport: this.#a
			}), this.dev = new io({
				browserId: e,
				tabId: this.id,
				transport: this.#a
			}), this.capabilities = jm({
				browserId: this.#e,
				capabilities: t,
				documentation: a,
				tabId: this.id,
				transport: this.#a
			});
		}
		async goto(e) {
			if (!e) throw new Error("tab.goto requires a url");
			if (this.#r(), !this.id) throw new Error("tab.goto requires a tab id");
			await this.#a.send({ command: ir({
				browser_id: this.#e,
				tab_id: this.id,
				url: e
			}) });
		}
		async markHandoff() {
			await this.#t("handoff");
		}
		async markDeliverable() {
			await this.#t("deliverable");
		}
		async requestManualHandoff() {
			return this.requestUserTakeover();
		}
		async requestUserTakeover(e) {
			await this.#a.send({ command: Ra({
				browser_id: this.#e,
				tab_id: this.id,
				reason: e
			}) });
		}
		async back() {
			this.#r(), await this.#a.send({ command: it({
				browser_id: this.#e,
				tab_id: this.id
			}) });
		}
		async forward() {
			this.#r(), await this.#a.send({ command: nt({
				browser_id: this.#e,
				tab_id: this.id
			}) });
		}
		async reload() {
			this.#r(), await this.#a.send({ command: hr({
				browser_id: this.#e,
				tab_id: this.id
			}) });
		}
		async close() {
			this.#r(), await this.#a.send({ command: ga({
				browser_id: this.#e,
				tab_id: this.id
			}) });
		}
		async screenshot(e = {}) {
			this.#r();
			const t = {
				browser_id: this.#e,
				tab_id: this.id,
				fullPage: e.fullPage
			};
			if (e.clip) {
				const { x: a, y: r, width: s, height: c } = e.clip;
				if (typeof a != "number" || typeof r != "number" || typeof s != "number" || typeof c != "number") throw new Error("tab.screenshot clip requires x, y, width, and height");
				t.cropX = a, t.cropY = r, t.cropWidth = s, t.cropHeight = c;
			}
			return Ee((await this.#a.send({ command: ns(t) })).data);
		}
		async title() {
			return (await this.#s()).title;
		}
		async url() {
			return (await this.#s()).url;
		}
		async getJsDialog() {
			this.#r();
			const e = (await this.#a.send({ command: Nr({
				browser_id: this.#e,
				tab_id: this.id
			}) })).dialog;
			if (e != null) return Zm({
				browserId: this.#e,
				dialogId: e.id,
				tabId: this.id,
				transport: this.#a,
				type: e.type
			});
		}
		async #t(e) {
			this.#r(), await this.#a.send({ command: xa({
				browser_id: this.#e,
				tab_id: this.id,
				status: e
			}) });
		}
		#r() {
			if (!this.#a) throw new Error("Tab is not bound to a transport");
		}
		async #s() {
			return this.#r(), await this.#a.send({ command: rt({
				browser_id: this.#e,
				tab_id: this.id
			}) });
		}
	};
	function Zm({ browserId: e, dialogId: t, tabId: a, transport: r, type: s }) {
		switch (s) {
			case "alert": return new ec({
				browserId: e,
				dialogId: t,
				tabId: a,
				transport: r
			});
			case "beforeunload": return new tc({
				browserId: e,
				dialogId: t,
				tabId: a,
				transport: r
			});
			case "confirm": return new ac({
				browserId: e,
				dialogId: t,
				tabId: a,
				transport: r
			});
			case "prompt": return new rc({
				browserId: e,
				dialogId: t,
				tabId: a,
				transport: r
			});
		}
	}
	var sc = l({
		PayloadSchema: () => Me,
		ResultSchema: () => ic,
		commandType: () => xt,
		create: () => nc
	});
	const xt = "browser_management_call", Me = n({
		args: g(Z()),
		browser_id: i(),
		method: i(),
		namespace: i()
	}), ic = n({ value: Z() });
	function nc(e) {
		return new h(xt, Me, e);
	}
	var oc = l({
		AuditChangeSchema: () => dc,
		PayloadSchema: () => cc,
		ResultSchema: () => lc,
		commandType: () => Tt,
		create: () => uc
	});
	const Tt = "browser_management_get_audit_trail", cc = Me.pick({ browser_id: !0 }), dc = Me.omit({ browser_id: !0 }).extend({
		before: n({
			bookmarks: g(n({
				id: i(),
				index: u().optional(),
				parentId: i().optional(),
				title: i(),
				url: i().optional()
			})),
			tabLayout: n({
				groups: g(n({
					collapsed: k(),
					color: i(),
					id: u(),
					title: i().optional(),
					windowId: u()
				})),
				tabs: g(n({
					autoDiscardable: k(),
					groupId: u(),
					id: u(),
					index: u(),
					pinned: k(),
					url: i().optional(),
					windowId: u()
				}))
			}),
			windows: g(n({
				focused: k(),
				height: u().optional(),
				id: u(),
				left: u().optional(),
				state: i().optional(),
				top: u().optional(),
				width: u().optional()
			}))
		}).partial(),
		createdAt: u(),
		result: Ht([u(), n({ id: i() })]).optional()
	}), lc = n({ changes: g(dc) });
	function uc(e) {
		return new h(Tt, cc, e);
	}
	var qm = l({
		Call: () => sc,
		GetAuditTrail: () => oc,
		commands: () => Um
	});
	const Um = {
		[xt]: sc,
		[Tt]: oc
	}, Fm = {
		id: "management",
		description: "Organize windows, tabs, tab groups, and bookmarks. Use only for user-requested browser organization."
	};
	var hc = class extends _t {
		constructor({ browserId: e, documentation: t, info: a, transport: r }) {
			super(r, e, t, a), this.windows = this.createNamespace("windows"), this.tabs = this.createNamespace("tabs"), this.tabGroups = this.createNamespace("tabGroups"), this.bookmarks = this.createNamespace("bookmarks");
		}
		async getAuditTrail() {
			const e = await this.transport.send({ command: uc({ browser_id: this.browserId }) });
			return lc.parse(e);
		}
		async invoke(e, t, ...a) {
			const r = await this.transport.send({ command: nc({
				args: a,
				browser_id: this.browserId,
				method: t,
				namespace: e
			}) });
			return ic.parse(r).value;
		}
		createNamespace(e) {
			return new Proxy({}, { get: (t, a) => {
				if (!(typeof a != "string" || a === "then" || a === "toJSON" || a in Object.prototype)) return (...r) => this.invoke(e, a, ...r);
			} });
		}
	};
	const mc = bt({
		capability: hc,
		info: Fm
	});
	var pc = l({
		PayloadSchema: () => fc,
		ResultSchema: () => yc,
		commandType: () => $t,
		create: () => _c
	});
	const $t = "browser_visibility_get", fc = n({ browser_id: i() }), yc = n({ visible: k() });
	function _c(e) {
		return new h($t, fc, e);
	}
	var bc = l({
		PayloadSchema: () => gc,
		ResultSchema: () => wc,
		commandType: () => kt,
		create: () => vc
	});
	const kt = "browser_visibility_set", gc = n({
		browser_id: i(),
		visible: k()
	}), wc = n({});
	function vc(e) {
		return new h(kt, gc, e);
	}
	var Vm = l({
		Get: () => pc,
		Set: () => bc,
		commands: () => Hm
	});
	const Hm = {
		[$t]: pc,
		[kt]: bc
	}, Wm = {
		id: "visibility",
		description: "Use to show or hide the browser to the user, and to determine the browser's current visibility. Keep browser work in the background unless the user asks to see it or live viewing is useful. When the browser should be visible, call set(true)."
	};
	var Sc = class extends _t {
		constructor({ browserId: e, documentation: t, info: a, transport: r }) {
			super(r, e, t, a);
		}
		async set(e) {
			const t = await this.transport.send({ command: vc({
				browser_id: this.browserId,
				visible: e
			}) });
			wc.parse(t);
		}
		async get() {
			const e = await this.transport.send({ command: _c({ browser_id: this.browserId }) });
			return yc.parse(e).visible;
		}
	};
	const xc = bt({
		capability: Sc,
		info: Wm
	});
	var Tc = l({
		PayloadSchema: () => $c,
		ResultSchema: () => kc,
		commandType: () => Rt,
		create: () => Rc
	});
	const Rt = "browser_viewport_reset", $c = n({ browser_id: i() }), kc = n({});
	function Rc(e) {
		return new h(Rt, $c, e);
	}
	var Cc = l({
		PayloadSchema: () => Ic,
		ResultSchema: () => Pc,
		commandType: () => Ct,
		create: () => Ec
	});
	const Ct = "browser_viewport_set", Ic = n({
		browser_id: i(),
		height: u().int().positive(),
		width: u().int().positive()
	}), Pc = n({});
	function Ec(e) {
		return new h(Ct, Ic, e);
	}
	var zm = l({
		Reset: () => Tc,
		Set: () => Cc,
		commands: () => Gm
	});
	const Gm = {
		[Rt]: Tc,
		[Ct]: Cc
	}, Jm = {
		id: "viewport",
		description: "Controls an explicit browser viewport override for responsive or device-size testing. Use it when a task calls for specific dimensions or breakpoint validation; otherwise leave it unset so the browser uses its normal viewport. Reset temporary overrides before finishing unless the user asked to keep them."
	};
	var Ac = class extends _t {
		constructor({ browserId: e, documentation: t, info: a, transport: r }) {
			super(r, e, t, a);
		}
		async set(e) {
			const t = await this.transport.send({ command: Ec({
				browser_id: this.browserId,
				...e
			}) });
			Pc.parse(t);
		}
		async reset() {
			const e = await this.transport.send({ command: Rc({ browser_id: this.browserId }) });
			kc.parse(e);
		}
	};
	const Nc = bt({
		capability: Ac,
		info: Jm
	}), Dc = [
		mc,
		xc,
		Nc
	], Km = Xm(Dc), Ym = ({ browserId: e, capabilities: t, documentation: a, transport: r }) => {
		const s = {};
		for (const c of t ?? []) {
			const o = Km.get(c.id);
			o != null && (s[c.id] = o({
				browserId: e,
				documentation: a,
				info: c,
				transport: r
			}));
		}
		return new no(s);
	};
	function Xm(e) {
		return new Map(e.map((t) => [t.id, t.create]));
	}
	var Oc = class {
		#e;
		#a;
		#t;
		constructor({ capabilities: e, onBrowserUsed: t, transport: a, browserId: r }) {
			this.#e = r, this.#a = t, this.#t = a, this.browserId = r;
			const s = new dt(a);
			this.capabilities = Ym({
				browserId: r,
				capabilities: e?.browser,
				documentation: s,
				transport: a
			});
			const c = (o) => new St({
				browserId: r,
				capabilities: e?.tab,
				documentation: s,
				transport: a,
				tabPayload: o
			});
			this.tabs = new Ln({
				browserId: r,
				createTab: c,
				transport: a
			}), this.user = new Zn({
				browserId: r,
				createTab: c,
				transport: a
			});
		}
		async documentation() {
			return this.#a?.(), await this.#t.send({ command: ca({ browser_id: this.browserId }) });
		}
		async history(e = {}) {
			if (e === null || Array.isArray(e) || typeof e != "object") throw new Error("browser.history expects an options object");
			const t = { browser_id: this.#e }, a = Qm(e.queries), r = ep(e.limit), s = Mc(e.from, "from"), c = Mc(e.to, "to");
			return a !== void 0 && (t.queries = a), r !== void 0 && (t.limit = r), s !== void 0 && (t.from = s), c !== void 0 && (t.to = c), (await this.#t.send({ command: $r(t) })).items;
		}
		async nameSession(e) {
			const t = e.trim();
			if (!t) throw new Error("browser.nameSession requires a name");
			await this.#t.send({ command: La({
				browser_id: this.browserId,
				name: t
			}) });
		}
	};
	const Mc = (e, t) => {
		if (e == null) return;
		const a = e instanceof Date ? e : new Date(e);
		if (Number.isNaN(a.getTime())) throw new Error(\`browser.history received an invalid \${t} date\`);
		return a.toISOString();
	}, Qm = (e) => {
		if (e == null) return;
		if (!Array.isArray(e) || e.length === 0) throw new Error("browser.history received invalid queries");
		const t = [];
		for (const a of e) {
			if (typeof a != "string") throw new Error("browser.history received invalid queries");
			t.push(a);
		}
		return t;
	}, ep = (e) => {
		if (e != null) {
			if (!Number.isInteger(e) || e <= 0) throw new Error("browser.history received an invalid limit");
			return e;
		}
	};
	var Bc = class {
		constructor({ executeAgentCommand: e, displaySideEffect: t } = {}) {
			if (typeof e != "function") throw new Error("FunctionAgentTransport requires an executeAgentCommand function");
			this.executeAgentCommand = e, this.displaySideEffect = t ?? rp;
		}
		async display(e) {
			await this.displaySideEffect(e);
		}
		async send({ command: e, timeoutMs: t }) {
			const a = e.toJSON(), r = await tp(await this.executeAgentCommand({
				...a,
				client_timeout_ms: typeof t == "number" && t > 0 ? t : void 0
			}), this.displaySideEffect);
			if (r == null) throw new Error("transport send returned empty response");
			return r;
		}
	};
	const tp = async (e, t) => {
		const a = ap.safeParse(e);
		if (!a.success) return e;
		for (const c of a.data.side_effects) await t(c);
		const { side_effects: r, ...s } = a.data;
		return s;
	}, ap = n({ side_effects: g(i()) }).passthrough(), rp = async (e) => {
		const t = globalThis.display;
		if (typeof t == "function") {
			await t(e);
			return;
		}
		console.log(e);
	};
	var sp = l({
		AXAPI: () => ao,
		Agent: () => jn,
		AlertDialog: () => ec,
		AtlasCommand: () => h,
		BeforeUnloadDialog: () => tc,
		BotDetectionTabCapability: () => uo,
		Browser: () => Oc,
		BrowserAuthTabCapability: () => No,
		BrowserManagementCommands: () => qm,
		BrowserUser: () => Zn,
		BrowserViewportCommands: () => zm,
		BrowserVisibilityCommands: () => Vm,
		Browsers: () => Bn,
		CUAAPI: () => to,
		CdpTabCapability: () => So,
		Commands: () => Wh,
		ConfirmDialog: () => ac,
		ContentAPI: () => ro,
		Documentation: () => dt,
		DomCUAAPI: () => Qn,
		FunctionAgentTransport: () => Bc,
		ManagementBrowserCapability: () => hc,
		PageAssetsTabCapability: () => Ho,
		PlaywrightAPI: () => Xn,
		PlaywrightDownload: () => Kn,
		PlaywrightFileChooser: () => Yn,
		PlaywrightFrameLocator: () => Jn,
		PlaywrightLocator: () => ue,
		PromptDialog: () => rc,
		SiwcAuthenticateOptionsSchema: () => Ca,
		SiwcCandidateSchema: () => le,
		SiwcDetectionResultSchema: () => Re,
		SiwcTabCapability: () => Yo,
		SiwcTargetSchema: () => st,
		Tab: () => St,
		TabBotDetectionCommands: () => im,
		TabBrowserAuthCommands: () => wm,
		TabCdpCommands: () => um,
		TabClipboardAPI: () => so,
		TabDevAPI: () => io,
		TabPageAssetsCommands: () => Nm,
		TabWebMcpCapability: () => Jo,
		Tabs: () => Ln,
		ViewportBrowserCapability: () => Ac,
		VisibilityBrowserCapability: () => Sc,
		botDetectionTabCapabilityDefinition: () => ho,
		browserAuthTabCapabilityDefinition: () => Do,
		browserCapabilityDefinitions: () => Dc,
		cdpTabCapabilityDefinition: () => xo,
		generateRequestId: () => zh,
		hasErrorMessage: () => Gh,
		isPlainObject: () => Fn,
		isRegExp: () => Ae,
		managementBrowserCapabilityDefinition: () => mc,
		pageAssetsTabCapabilityDefinition: () => Wo,
		siwcTabCapabilityDefinition: () => Xo,
		tabCapabilityDefinitions: () => Qo,
		toWebMcpToolDescriptor: () => Go,
		viewportBrowserCapabilityDefinition: () => Nc,
		visibilityBrowserCapabilityDefinition: () => xc,
		webMcpTabCapabilityDefinition: () => Ko,
		webMcpTabCapabilityInfo: () => zo
	});
	function ip({ apiManifest: e, browserInfo: t, disabledMemberIds: a }) {
		const r = new Set(a);
		for (const [s, c] of Object.entries(e.interfaces)) for (const [o, d] of Object.entries(c)) {
			const p = \`\${s}.\${o}\`;
			(t.apiSupportOverrides?.[p] ?? d.unsupportedByDefaultIn?.includes(t.type) !== !0) || r.add(p);
		}
		return r;
	}
	var np = class {
		apiManifest;
		disabledMemberIds;
		view;
		constructor({ apiManifest: e, decorateTab: t, disabledMemberIds: a }) {
			this.apiManifest = e, this.disabledMemberIds = a, this.view = op(e, t);
		}
		createBrowser = ({ browserInfo: e, onBrowserUsed: t, transport: a }) => {
			const r = ip({
				apiManifest: this.apiManifest,
				browserInfo: e,
				disabledMemberIds: this.disabledMemberIds
			});
			return this.view(new Oc({
				browserId: e.id,
				capabilities: e.capabilities,
				onBrowserUsed: t,
				transport: a
			}), r);
		};
		wrapAgent(e) {
			return this.view(e, this.disabledMemberIds);
		}
		createAgent({ displaySideEffect: e, executeAgentCommand: t }) {
			return this.wrapAgent(new jn({
				createBrowser: this.createBrowser,
				transport: new Bc({
					displaySideEffect: e,
					executeAgentCommand: t
				})
			}));
		}
	};
	function op(e, t) {
		const a = /* @__PURE__ */ new WeakMap(), r = Object.keys(e.interfaces).map((s) => {
			const c = Reflect.get(sp, s);
			if (!cp(c)) throw new Error(\`Browser API interface has no runtime type: \${s}\`);
			return [c, s];
		});
		return (s, c) => {
			const o = /* @__PURE__ */ new WeakMap(), d = (f) => {
				if (f == null || typeof f != "object") return f;
				return a.get(f) ?? (Array.isArray(f) ? f.map(d) : Fn(f) ? Object.fromEntries(Object.entries(f).map(([P, D]) => [P, d(D)])) : f);
			}, p = (f) => {
				if (f == null || typeof f != "object" || a.has(f)) return f;
				if (f instanceof Promise) return f.then(p);
				if (Array.isArray(f)) return f.map(p);
				const R = r.find(([A]) => f instanceof A)?.[1];
				if (R == null) return f;
				const P = o.get(f);
				if (P != null) return P;
				const D = (A) => typeof A != "string" || !c.has(\`\${R}.\${A}\`), Zc = /* @__PURE__ */ new Map(), he = new Proxy(f, {
					get(A, M) {
						if (!D(M)) return;
						const U = Reflect.get(A, M, A);
						if (M === "constructor") return U;
						if (typeof U != "function") return p(U);
						const qc = Zc.get(M);
						if (qc != null) return qc;
						const Uc = (...fp) => p(Reflect.apply(U, A, fp.map((yp) => d(yp))));
						return Zc.set(M, Uc), Uc;
					},
					getOwnPropertyDescriptor(A, M) {
						if (!D(M)) return;
						const U = Reflect.getOwnPropertyDescriptor(A, M);
						return U == null || !("value" in U) ? U : {
							...U,
							value: p(U.value)
						};
					},
					has(A, M) {
						return D(M) && Reflect.has(A, M);
					},
					ownKeys(A) {
						return Reflect.ownKeys(A).filter(D);
					}
				});
				return o.set(f, he), a.set(he, f), he instanceof St && t?.(he), he;
			};
			return p(s);
		};
	}
	function cp(e) {
		return typeof e == "function" && e.prototype != null;
	}
	function dp({ apiManifest: e, decorateTab: t, disabledMemberIds: a, displayBridge: r, executeAgentCommand: s }) {
		return new np({
			apiManifest: e,
			decorateTab: t,
			disabledMemberIds: a
		}).createAgent({
			displaySideEffect: ed({
				displayBridge: r,
				displayTruncateMaxChars: 1e5
			}),
			executeAgentCommand: s
		});
	}
	Object.freeze(["http:", "https:"]);
	(function(e) {
		return e.Default = "", e.DisabledForLocalTesting = "disabled-for-local-testing", e.GaasBrowserEnvironment = "gaas-browser-environment", e;
	})({}).Default;
	new TextEncoder();
	const Be = {
		distribution: "chrome-web-store",
		extensionIds: ["lfkehkpjohcoelkpembgemeipeppanef"],
		outputModes: [],
		releaseChannel: "beta",
		signingIdentity: null,
		storeUrl: "https://chromewebstore.google.com/detail/codex/lfkehkpjohcoelkpembgemeipeppanef"
	}, jc = {
		distribution: "chrome-web-store",
		extensionIds: ["hehggadaopoacecdllhhajmbjkdcmajg"],
		outputModes: [],
		releaseChannel: "stable",
		signingIdentity: null,
		storeUrl: "https://chromewebstore.google.com/detail/chatgpt/hehggadaopoacecdllhhajmbjkdcmajg"
	}, Lc = {
		distribution: "microsoft-edge-add-ons",
		extensionIds: ["odlomjlbamekndcpllcnffbgeohgkmjh"],
		outputModes: [],
		releaseChannel: "stable",
		signingIdentity: null,
		storeUrl: "https://microsoftedge.microsoft.com/addons/detail/odlomjlbamekndcpllcnffbgeohgkmjh"
	}, se = {
		chrome: {
			dev: [
				{
					distribution: "local-development",
					extensionIds: [],
					outputModes: ["file"],
					releaseChannel: "dev",
					signingIdentity: "development"
				},
				{
					distribution: "managed-self-hosted",
					extensionIds: [],
					outputModes: ["file"],
					releaseChannel: "dev",
					signingIdentity: "development"
				},
				Be
			],
			internal: [{
				distribution: "managed-self-hosted",
				extensionIds: [],
				outputModes: ["file", "blobstore"],
				releaseChannel: "beta",
				signingIdentity: "beta"
			}, Be],
			prod: [{
				distribution: "managed-self-hosted",
				extensionIds: [],
				outputModes: ["file"],
				releaseChannel: "stable",
				signingIdentity: "stable"
			}, jc]
		},
		edge: {
			dev: [
				{
					distribution: "local-development",
					extensionIds: [],
					outputModes: ["file"],
					releaseChannel: "dev",
					signingIdentity: "development"
				},
				{
					distribution: "managed-self-hosted",
					extensionIds: [],
					outputModes: ["file"],
					releaseChannel: "dev",
					signingIdentity: "development"
				},
				Be
			],
			internal: [
				{
					distribution: "managed-self-hosted",
					extensionIds: [],
					outputModes: ["file", "blobstore"],
					releaseChannel: "beta",
					signingIdentity: "beta"
				},
				Lc,
				Be
			],
			prod: [
				{
					distribution: "managed-self-hosted",
					extensionIds: [],
					outputModes: ["file"],
					releaseChannel: "stable",
					signingIdentity: "stable"
				},
				Lc,
				jc
			]
		}
	};
	se.chrome.dev, se.chrome.internal, se.chrome.prod;
	se.edge.dev, se.edge.internal, se.edge.prod;
	var lp = class {
		closed = !1;
		nextId = 1;
		pendingRequests = /* @__PURE__ */ new Map();
		requestHandlers = /* @__PURE__ */ new Map();
		eventHandlers = /* @__PURE__ */ new Map();
		constructor(e) {
			this.transport = e, this.transport.setMessageCallback((t) => {
				this.handleIncomingMessage(t);
			}), this.transport.addCloseListener?.((t) => {
				this.closed = !0, this.rejectPendingRequests(t?.message ?? "transport closed before response");
			});
		}
		registerRequestHandlerObject(e) {
			[...Object.getOwnPropertyNames(e), ...Object.getOwnPropertyNames(Object.getPrototypeOf(e))].filter((t) => t !== "constructor" && typeof e[t] == "function").forEach((t) => {
				const a = e[t];
				a && this.registerRequestHandler(t, a.bind(e));
			});
		}
		registerRequestHandler(e, t) {
			this.requestHandlers.set(e, t);
		}
		addEventListener(e, t) {
			const a = e.toString(), r = this.eventHandlers.get(a) ?? [];
			r.push(t), this.eventHandlers.set(a, r);
		}
		removeEventListener(e, t) {
			const a = e.toString(), r = this.eventHandlers.get(a) ?? [];
			this.eventHandlers.set(a, r.filter((s) => s !== t));
		}
		sendNotification(e, t) {
			this.transport.sendMessage({
				jsonrpc: "2.0",
				method: e,
				params: t
			});
		}
		sendRequest(e, t) {
			const a = this.nextId++;
			return new Promise((r, s) => {
				this.pendingRequests.set(a, {
					resolve: r,
					reject: s
				});
				try {
					this.transport.sendMessage({
						jsonrpc: "2.0",
						method: e.toString(),
						params: t,
						id: a
					});
				} catch (c) {
					this.pendingRequests.delete(a), s(c);
				}
			});
		}
		async handleIncomingMessage(e) {
			if ("method" in e) return this.handleIncomingRequest(e);
			if (e.id === void 0) return;
			const t = this.pendingRequests.get(e.id);
			if (t) {
				if (this.pendingRequests.delete(e.id), "error" in e) {
					t.reject(e.error?.message || "Something went wrong");
					return;
				}
				t.resolve(e.result);
			}
		}
		rejectPendingRequests(e) {
			for (const t of this.pendingRequests.values()) t.reject(e);
			this.pendingRequests.clear();
		}
		async handleIncomingRequest(e) {
			if (e.id === void 0) {
				(this.eventHandlers.get(e.method ?? "") ?? []).forEach((a) => a(e.params));
				return;
			}
			const t = this.requestHandlers.get(e.method ?? "");
			if (!t) {
				this.transport.sendMessage({
					jsonrpc: "2.0",
					id: e.id,
					error: {
						code: -1,
						message: \`No handler registered for method: \${e.method}\`
					}
				});
				return;
			}
			try {
				const a = await t(e.params);
				if (this.closed) return;
				this.transport.sendMessage({
					jsonrpc: "2.0",
					id: e.id,
					result: a
				});
			} catch (a) {
				if (this.closed) return;
				this.transport.sendMessage({
					jsonrpc: "2.0",
					id: e.id,
					error: {
						code: 1,
						message: a instanceof Error ? a.message : String(a)
					}
				});
			}
		}
	}, up = class extends lp {
		constructor(e) {
			super(e), this.setup = this.setup.bind(this), this.execute = this.execute.bind(this), this.display = this.display.bind(this);
		}
		setup() {
			return this.sendRequest("setup");
		}
		execute(e) {
			return this.sendRequest("execute", e);
		}
		display(e) {
			return this.sendRequest("display", e);
		}
	}, hp = class {
		closeListeners = /* @__PURE__ */ new Set();
		closed = !1;
		messageCallback;
		constructor(e) {
			this.port = e, this.port.onmessage = (t) => {
				this.messageCallback?.(t.data);
			}, this.port.onmessageerror = () => {
				this.finish(/* @__PURE__ */ new Error("JavaScript kernel agent port received invalid data"));
			}, this.port.start();
		}
		sendMessage(e) {
			if (this.closed) throw new Error("JavaScript kernel agent transport is closed");
			this.port.postMessage(e);
		}
		setMessageCallback(e) {
			this.messageCallback = e;
		}
		addCloseListener(e) {
			return this.closeListeners.add(e), () => {
				this.closeListeners.delete(e);
			};
		}
		async close() {
			this.finish();
		}
		finish(e) {
			if (!this.closed) {
				this.closed = !0, this.port.close();
				for (const t of this.closeListeners) Promise.resolve(t(e)).catch(() => {});
				this.closeListeners.clear();
			}
		}
	};
	function mp(e) {
		return e instanceof Error ? {
			message: e.message,
			name: e.name,
			stack: e.stack
		} : {
			message: typeof e == "string" ? e : "JavaScript execution failed",
			name: "Error"
		};
	}
	function pp() {
		const e = globalThis;
		let t = { evaluate: globalThis.eval }, a = Promise.resolve();
		e.addEventListener("message", (r) => {
			if (r.data.type === "js-kernel:agent-port") {
				const o = r.ports[0];
				if (o == null) return;
				const d = new up(new hp(o)), p = async (R) => {
					await d.display(R);
				}, f = async (R) => {
					await d.display(R);
				};
				e.nodeRepl = { write: p }, Object.assign(e, { display: p }), e.setupBrowserRuntime = async () => {
					const { apiManifest: R, disabledMemberIds: P } = await d.setup();
					return dp({
						apiManifest: R,
						disabledMemberIds: new Set(P),
						displayBridge: {
							displayImage: f,
							displayValue: p
						},
						executeAgentCommand: (D) => d.execute(D)
					});
				};
				return;
			}
			const { id: s, script: c } = r.data;
			a = a.then(async () => {
				try {
					let o = t;
					await t.evaluate(\`(async function () {
arguments[0]({ evaluate(source) { return eval(source); } });
\${c}
})\`)((d) => {
						o = d;
					}), t = o, e.postMessage({
						id: s,
						ok: !0
					});
				} catch (o) {
					e.postMessage({
						error: mp(o),
						id: s,
						ok: !1
					});
				}
			});
		});
	}
	pp();
})();
`,o=typeof self<"u"&&self.Blob&&new Blob(["(self.URL || self.webkitURL).revokeObjectURL(self.location.href);",c],{type:"text/javascript;charset=utf-8"});function h(n){let e;try{if(e=o&&(self.URL||self.webkitURL).createObjectURL(o),!e)throw"";const t=new Worker(e,{name:n?.name});return t.addEventListener("error",()=>{(self.URL||self.webkitURL).revokeObjectURL(e)}),t}catch{return new Worker("data:text/javascript;charset=utf-8,"+encodeURIComponent(c),{name:n?.name})}}var m=class{pending=new Map;nextRequestId=0;resetError;worker=null;constructor(n){this.onAgentPort=n}executeJs(n){if(this.resetError!=null)return Promise.reject(this.resetError);const e=this.getWorker(),t=String(this.nextRequestId++);return new Promise((s,a)=>{this.pending.set(t,{reject:a,resolve:s});try{e.postMessage({id:t,script:n,type:"js-kernel:execute"})}catch(r){this.fail(r)}})}getWorker(){if(this.worker!=null)return this.worker;const n=new h;n.addEventListener("message",t=>{l(t.data)&&this.handleResponse(t.data)}),n.addEventListener("error",t=>{this.fail(new Error(t.message||"JavaScript worker failed"))}),n.addEventListener("messageerror",()=>{this.fail(new Error("JavaScript worker returned an invalid result"))});const e=new MessageChannel;return n.postMessage({type:"js-kernel:agent-port"},[e.port1]),this.onAgentPort(e.port2),this.worker=n,n}handleResponse(n){const e=this.pending.get(n.id);if(e==null)return;if(this.pending.delete(n.id),n.ok){e.resolve();return}const t=new Error(n.error?.message??"JavaScript execution failed");t.name=n.error?.name??"Error",n.error?.stack!=null&&(t.stack=n.error.stack),e.reject(t)}fail(n){this.worker?.terminate(),this.worker=null;const e=new Error(`${n instanceof Error?n.message:"JavaScript worker stopped"}; this task's JavaScript kernel will be reset.`);e.name="JsKernelWorkerResetError",this.resetError=e;for(const t of this.pending.values())t.reject(e);this.pending.clear()}},p=new m(n=>{window.parent.postMessage({type:"js-kernel:agent-port"},"*",[n])});function f(){window.addEventListener("message",n=>{if(n.source!==window.parent||!d(n.data)||n.data.type!=="js-kernel:execute")return;const e=n.ports[0];e!=null&&_(n.data,e)}),window.parent.postMessage({type:"js-kernel:ready"},"*")}async function _(n,e){try{await p.executeJs(n.script),e.postMessage({id:n.id,ok:!0})}catch(t){e.postMessage({error:u(t),id:n.id,ok:!1})}finally{e.close()}}f();
