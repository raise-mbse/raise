use crate::code_generator::models::{CodeElement, CodeElementType, Visibility};
use crate::utils::prelude::*;

// =========================================================================
// 1. LE LEXER ZERO-COPY (TOKENIZER)
// Rôle : Découper le texte brut en morceaux typés sans allocation inutile.
// =========================================================================

#[derive(Debug, PartialEq, Clone)]
enum Token<'a> {
    Ident(&'a str),
    Symbol(char),
    StringLit(&'a str),
    RawStringLit(&'a str),
    CharLit(&'a str),
    Lifetime(&'a str), // Support des lifetimes ('a, 'static)
    LineComment(&'a str),
    BlockComment(&'a str),
    Whitespace(&'a str),
}

impl<'a> Token<'a> {
    fn as_str(&self) -> &'a str {
        match self {
            Token::Ident(s)
            | Token::StringLit(s)
            | Token::RawStringLit(s)
            | Token::CharLit(s)
            | Token::Lifetime(s) // 🎯 NOUVEAU
            | Token::LineComment(s)
            | Token::BlockComment(s)
            | Token::Whitespace(s) => s,
            Token::Symbol(_) => "",
        }
    }
}

struct Lexer<'a> {
    source: &'a str,
    chars: DataStreamPeekable<TextCharIndices<'a>>,
}

impl<'a> Lexer<'a> {
    fn new(input: &'a str) -> Self {
        Self {
            source: input,
            chars: input.char_indices().peekable(),
        }
    }

    /// Récupère l'index de l'octet courant
    fn current_index(&mut self) -> usize {
        self.chars
            .peek()
            .map(|&(i, _)| i)
            .unwrap_or(self.source.len())
    }

    fn tokenize(&mut self) -> Vec<Token<'a>> {
        let mut tokens = Vec::new();

        while let Some(&(start_idx, c)) = self.chars.peek() {
            match c {
                c if c.is_whitespace() => tokens.push(self.read_whitespace(start_idx)),
                c if c.is_alphabetic() || c == '_' => {
                    // 🎯 FIX : Détection des Raw Strings (ex: r#"..."#)
                    if c == 'r' {
                        let mut lookahead = self.chars.clone();
                        lookahead.next(); // Passe le 'r'
                        if let Some(&(_, next_c)) = lookahead.peek() {
                            if next_c == '#' || next_c == '"' {
                                if let Some(tok) = self.read_raw_string(start_idx) {
                                    tokens.push(tok);
                                    continue;
                                }
                            }
                        }
                    }
                    tokens.push(self.read_ident(start_idx))
                }
                '"' => tokens.push(self.read_string_lit(start_idx)),
                '\'' => {
                    // 🎯 FIX : Différencier un CharLit ('a') d'une Lifetime ('a)
                    let mut lookahead = self.chars.clone();
                    lookahead.next(); // Passe le '\''
                    let mut is_lifetime = false;

                    if let Some(&(_, c1)) = lookahead.peek() {
                        if c1.is_alphabetic() || c1 == '_' {
                            lookahead.next();
                            if let Some(&(_, c2)) = lookahead.peek() {
                                if c2 != '\'' {
                                    is_lifetime = true;
                                }
                            } else {
                                is_lifetime = true;
                            }
                        }
                    }

                    if is_lifetime {
                        tokens.push(self.read_lifetime(start_idx));
                    } else {
                        tokens.push(self.read_char_lit(start_idx));
                    }
                }
                '/' => {
                    self.chars.next(); // Consomme le 1er '/'
                    match self.chars.peek() {
                        Some(&(_, '/')) => tokens.push(self.read_line_comment(start_idx)),
                        Some(&(_, '*')) => tokens.push(self.read_block_comment(start_idx)),
                        _ => tokens.push(Token::Symbol('/')),
                    }
                }
                _ => {
                    tokens.push(Token::Symbol(c));
                    self.chars.next();
                }
            }
        }
        tokens
    }

    fn read_whitespace(&mut self, start: usize) -> Token<'a> {
        while let Some(&(_, c)) = self.chars.peek() {
            if c.is_whitespace() {
                self.chars.next();
            } else {
                break;
            }
        }
        Token::Whitespace(&self.source[start..self.current_index()])
    }

    fn read_ident(&mut self, start: usize) -> Token<'a> {
        while let Some(&(_, c)) = self.chars.peek() {
            if c.is_alphanumeric() || c == '_' {
                self.chars.next();
            } else {
                break;
            }
        }
        Token::Ident(&self.source[start..self.current_index()])
    }

    fn read_string_lit(&mut self, start: usize) -> Token<'a> {
        self.chars.next(); // '"'
        while let Some(&(_, c)) = self.chars.peek() {
            self.chars.next();
            if c == '\\' {
                self.chars.next();
            }
            // Skip escaped char
            else if c == '"' {
                break;
            }
        }
        Token::StringLit(&self.source[start..self.current_index()])
    }

    fn read_lifetime(&mut self, start: usize) -> Token<'a> {
        self.chars.next(); // '\''
        while let Some(&(_, c)) = self.chars.peek() {
            if c.is_alphanumeric() || c == '_' {
                self.chars.next();
            } else {
                break;
            }
        }
        Token::Lifetime(&self.source[start..self.current_index()])
    }

    fn read_raw_string(&mut self, start: usize) -> Option<Token<'a>> {
        self.chars.next(); // 'r'
        let mut hashes = 0;

        while let Some(&(_, c)) = self.chars.peek() {
            if c == '#' {
                hashes += 1;
                self.chars.next();
            } else if c == '"' {
                self.chars.next();
                break;
            } else {
                return None;
            } // Invalide
        }

        while let Some(&(_, c)) = self.chars.peek() {
            self.chars.next();
            if c == '"' {
                let mut closing_hashes = 0;
                let mut lookahead = self.chars.clone();
                for _ in 0..hashes {
                    if let Some(&(_, '#')) = lookahead.peek() {
                        closing_hashes += 1;
                        lookahead.next();
                    }
                }
                if closing_hashes == hashes {
                    for _ in 0..hashes {
                        self.chars.next();
                    } // Consomme les # de fin
                    return Some(Token::RawStringLit(
                        &self.source[start..self.current_index()],
                    ));
                }
            }
        }
        None
    }

    fn read_char_lit(&mut self, start: usize) -> Token<'a> {
        self.chars.next(); // '\''
        while let Some(&(_, c)) = self.chars.peek() {
            self.chars.next();
            if c == '\\' {
                self.chars.next();
            } else if c == '\'' {
                break;
            }
        }
        Token::CharLit(&self.source[start..self.current_index()])
    }

    fn read_line_comment(&mut self, start: usize) -> Token<'a> {
        while let Some(&(_, c)) = self.chars.peek() {
            if c == '\n' {
                break;
            }
            self.chars.next();
        }
        Token::LineComment(&self.source[start..self.current_index()])
    }

    fn read_block_comment(&mut self, start: usize) -> Token<'a> {
        self.chars.next(); // '*'
        let mut prev = '\0';
        while let Some(&(_, c)) = self.chars.peek() {
            self.chars.next();
            if prev == '*' && c == '/' {
                break;
            }
            prev = c;
        }
        Token::BlockComment(&self.source[start..self.current_index()])
    }
}

// =========================================================================
// 2. LE PARSER (AST SHALLOW EXTRACTOR)
// =========================================================================

pub struct Reconciler;

impl Reconciler {
    pub async fn parse_from_file(path: &Path, module_id: String) -> RaiseResult<Vec<CodeElement>> {
        let content = match fs::read_to_string_async(path).await {
            Ok(c) => c,
            Err(e) => raise_error!(
                "ERR_SYSTEM_IO",
                error = e,
                context = json_value!({ "action": "read_file_async", "path": path.display().to_string() })
            ),
        };
        Self::parse_content(&content, module_id)
    }

    pub fn parse_content(content: &str, module_id: String) -> RaiseResult<Vec<CodeElement>> {
        let mut lexer = Lexer::new(content);
        let tokens = lexer.tokenize();

        // =====================================================================
        // 🕵️ EXTRACTION SILENCIEUSE DES DÉPENDANCES (Imports)
        // =====================================================================
        let mut file_dependencies = Vec::new();
        let mut j = 0;
        let mut current_brace_depth: usize = 0;

        while j < tokens.len() {
            match &tokens[j] {
                Token::Symbol('{') => current_brace_depth += 1,
                Token::Symbol('}') => current_brace_depth = current_brace_depth.saturating_sub(1),
                Token::Ident(kw) if *kw == "use" && current_brace_depth == 0 => {
                    let mut dep_str = String::new();
                    let mut k = j + 1;
                    while k < tokens.len() {
                        match &tokens[k] {
                            Token::Symbol(';') => break,
                            Token::Symbol(c) => dep_str.push(*c),
                            Token::Whitespace(_) => dep_str.push(' '),
                            t => dep_str.push_str(t.as_str()),
                        }
                        k += 1;
                    }

                    // Nettoyage esthétique (retire les espaces superflus autour des ::)
                    let clean_dep = dep_str
                        .trim()
                        .replace(" :: ", "::")
                        .replace(":: ", "::")
                        .replace(" ::", "::");

                    file_dependencies.push(clean_dep);
                    j = k; // On avance directement à la fin de l'import
                }
                _ => {}
            }
            j += 1;
        }
        // =====================================================================

        let mut elements = Vec::new();
        // 🎯 ÉTAPE 1 : Génération du bloc d'imports pour le Weaver
        if !file_dependencies.is_empty() {
            let imports_body = file_dependencies
                .iter()
                .map(|d| format!("use {};", d))
                .collect::<Vec<_>>()
                .join("\n");

            elements.push(CodeElement {
                module_id: Some(module_id.clone()),
                parent_id: None, // C'est un élément racine du fichier
                element_type: CodeElementType::ImportBlock,
                handle: "sys:imports".to_string(), // Handle système réservé
                visibility: Visibility::Private,
                attributes: vec![],
                docs: None,
                signature: "".to_string(),
                body: Some(imports_body),
                elements: vec![],
                dependencies: vec![],
                metadata: UnorderedMap::new(),
            });
        }
        let mut i = 0;
        // 🎯 ÉTAPE 2 : Suivi de la topologie spatiale (Parent/Enfant)
        let mut current_brace_depth: usize = 0;
        let mut active_parents: Vec<(usize, String)> = Vec::new();

        while i < tokens.len() {
            // 1. Suivi strict de la profondeur lexicale
            if let Token::Symbol('{') = &tokens[i] {
                current_brace_depth += 1;
            } else if let Token::Symbol('}') = &tokens[i] {
                current_brace_depth = current_brace_depth.saturating_sub(1);
                // Si on remonte au-dessus du niveau du parent actuel, on dépile (fin de scope)
                if let Some(last) = active_parents.last() {
                    if current_brace_depth < last.0 {
                        active_parents.pop();
                    }
                }
            }

            // 2. Détection et extraction des éléments
            if let Token::LineComment(comment) = &tokens[i] {
                if comment.starts_with("// @raise-handle:") {
                    let full_tag = comment.replace("// @raise-handle:", "").trim().to_string();

                    // 🎯 RÈGLE D'ARCHITECTURE : Les UUID/ID sont pour la DB.
                    // On extrait exclusivement le vrai handle sémantique (ex: impl:IndustrialPhase)
                    let handle = if let Some(start) = full_tag.find(" [id:") {
                        full_tag[..start].trim().to_string()
                    } else if let Some(start) = full_tag.find("[id:") {
                        full_tag[..start].trim().to_string()
                    } else {
                        full_tag.clone()
                    };

                    i += 1; // Pointe sur le début de l'élément (docs, attributs ou signature)

                    // 🎯 FIX : On extrait l'élément mutablement pour pouvoir y injecter nos dépendances
                    let (mut element, _next_index) =
                        Self::extract_element(&handle, &tokens, i, module_id.clone())?;

                    // 🎯 INJECTION DE LA TOPOLOGIE : Assignation du parent sémantique
                    if let Some(parent) = active_parents.last() {
                        element.parent_id = Some(parent.1.clone());
                        // 🛡️ PARADE ANTI-RÉÉCRITURE : On sauvegarde le handle textuel pur dans les métadonnées.
                        // Ainsi, si JsonDb convertit 'parent_id' en UUID interne, on aura toujours cette ancre de secours.
                        element
                            .metadata
                            .insert("semantic_parent_handle".to_string(), parent.1.clone());
                    }

                    // 🎯 Si l'élément est un conteneur physique, il devient le parent actif
                    if element.element_type == CodeElementType::ImplBlock
                        || element.element_type == CodeElementType::TestModule
                    {
                        // On enregistre la profondeur actuelle + 1 (l'intérieur du bloc)
                        active_parents.push((current_brace_depth + 1, handle.clone()));
                    }

                    // 💉 INJECTION DES DÉPENDANCES
                    // Le composant hérite des imports de niveau module pour son contexte IA
                    if !file_dependencies.is_empty() {
                        element
                            .metadata
                            .insert("raw_imports".to_string(), file_dependencies.join(","));
                    }

                    // 🎯 CHRONOLOGIE : Mémoriser la position physique exacte du code source
                    let seq_index = elements.len().to_string();
                    element
                        .metadata
                        .insert("physical_index".to_string(), seq_index);

                    elements.push(element);

                    // 🚀 On laisse la boucle continuer naturellement à `i`.
                    // Le parseur va donc traverser l'intérieur des `impl` et découvrir les sous-tags !
                    continue;
                }
            }
            i += 1;
        }

        Ok(elements)
    }

    fn extract_element(
        handle: &str,
        tokens: &[Token],
        start_index: usize,
        module_id: String,
    ) -> RaiseResult<(CodeElement, usize)> {
        let mut i = start_index;
        let mut docs = String::new();
        let mut attributes = Vec::new();

        // 1. Extraction des Métadonnées (Docs et Attributs)
        while i < tokens.len() {
            match &tokens[i] {
                Token::Whitespace(_) => i += 1,
                Token::LineComment(c) if c.starts_with("///") => {
                    docs.push_str(c.trim_start_matches("///").trim());
                    docs.push('\n');
                    i += 1;
                }
                Token::Symbol('#') => {
                    let mut attr_str = String::new();
                    let mut bracket_count = 0;
                    let mut in_attr = false;

                    while i < tokens.len() {
                        let t = &tokens[i];
                        if let Token::Symbol(sym) = t {
                            attr_str.push(*sym);
                        } else {
                            attr_str.push_str(t.as_str());
                        }

                        if t == &Token::Symbol('[') {
                            bracket_count += 1;
                            in_attr = true;
                        } else if t == &Token::Symbol(']') {
                            bracket_count -= 1;
                        }

                        i += 1;
                        if in_attr && bracket_count == 0 {
                            break;
                        }
                    }
                    attributes.push(attr_str.trim().to_string());
                }
                _ => break, // On a atteint la signature
            }
        }

        // 2. Extraction de la Signature (Ignorant les commentaires normaux)
        let mut signature_str = String::new();
        let mut body_start_index = None;
        let mut has_body = false;

        while i < tokens.len() {
            let t = &tokens[i];

            // 🎯 FIX : On ignore les commentaires intra-signature (ex: /* arg */)
            if let Token::BlockComment(_) = t {
                i += 1;
                continue;
            }
            if let Token::LineComment(c) = t {
                if !c.starts_with("///") {
                    i += 1;
                    continue;
                }
            }

            if t == &Token::Symbol('{') {
                body_start_index = Some(i);
                has_body = true;
                break;
            } else if t == &Token::Symbol(';') {
                signature_str.push(';');
                i += 1;
                break;
            } else {
                if let Token::Symbol(sym) = t {
                    signature_str.push(*sym);
                } else {
                    signature_str.push_str(t.as_str());
                }
                i += 1;
            }
        }

        let signature = signature_str.trim().to_string();

        let visibility = if signature.starts_with("pub(crate)") {
            Visibility::Crate
        } else if signature.starts_with("pub ") {
            Visibility::Public
        } else {
            Visibility::Private
        };

        let element_type = if signature.contains("fn ") {
            CodeElementType::Function
        } else if signature.contains("struct ") {
            CodeElementType::Struct
        } else if signature.contains("trait ") {
            CodeElementType::Trait
        } else if signature.contains("impl ") || signature.contains("impl<") {
            CodeElementType::ImplBlock
        } else if signature.contains("enum ") {
            CodeElementType::Enum
        } else if signature.contains("macro_rules! ") {
            CodeElementType::Macro
        } else if signature.contains("mod ") {
            CodeElementType::TestModule
        } else {
            CodeElementType::Function
        };

        // 3. Extraction du Corps (Body)
        let mut body = None;
        let mut internal_dependencies = Vec::new();
        if has_body {
            if let Some(mut start) = body_start_index {
                let mut brace_count = 0;
                let mut body_str = String::new();

                while start < tokens.len() {
                    let t = &tokens[start];

                    // 🎯 INJECTION : Capture des dépendances internes au bloc
                    // On ne capture que les imports de premier niveau (brace_count == 1)
                    if let Token::Ident(kw) = t {
                        if *kw == "use" && brace_count == 1 {
                            let mut dep_str = String::new();
                            let mut scan_idx = start + 1;
                            while scan_idx < tokens.len() {
                                match &tokens[scan_idx] {
                                    Token::Symbol(';') => break,
                                    Token::Symbol(c) => dep_str.push(*c),
                                    Token::Whitespace(_) => dep_str.push(' '),
                                    tok => dep_str.push_str(tok.as_str()),
                                }
                                scan_idx += 1;
                            }
                            // Nettoyage esthétique de la chaîne d'import
                            let clean_dep = dep_str
                                .trim()
                                .replace(" :: ", "::")
                                .replace(":: ", "::")
                                .replace(" ::", "::");
                            internal_dependencies.push(clean_dep);
                        }
                    }

                    // Reconstruction du corps brut
                    if let Token::Symbol(sym) = t {
                        body_str.push(*sym);
                        if *sym == '{' {
                            brace_count += 1;
                        }
                        if *sym == '}' {
                            brace_count -= 1;
                        }
                    } else {
                        body_str.push_str(t.as_str());
                    }

                    start += 1;
                    if brace_count == 0 {
                        break;
                    }
                }

                if brace_count != 0 {
                    raise_error!(
                        "ERR_RECONCILER_UNBALANCED_BRACES",
                        error = "Accolades non équilibrées détectées dans le corps de l'élément.",
                        context = json_value!({ "handle": handle })
                    );
                }
                body = Some(body_str.trim().to_string());
                i = start;
            }
        }

        let mut element = CodeElement {
            module_id: Some(module_id),
            parent_id: None,
            element_type,
            handle: handle.to_string(),
            visibility,
            attributes,
            docs: if docs.is_empty() {
                None
            } else {
                Some(docs.trim().to_string())
            },
            signature,
            body,
            elements: Vec::new(),
            dependencies: Vec::new(),
            metadata: UnorderedMap::new(),
        };

        if !internal_dependencies.is_empty() {
            element.metadata.insert(
                "internal_imports".to_string(),
                internal_dependencies.join(","),
            );
        }

        Ok((element, i))
    }

    /// 🚀 AUTO-TAGGING & GARBAGE COLLECTOR : Injecte, corrige et nettoie les ancres sémantiques.
    pub async fn auto_tag_module(module_doc: &JsonValue) -> RaiseResult<usize> {
        // --- 1. RÉSOLUTION SÉMANTIQUE ---
        let handle = module_doc
            .get("handle")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown_module");
        let module_id = module_doc
            .get("_id")
            .and_then(|v| v.as_str())
            .unwrap_or("no-id");
        let path_str = match module_doc.get("path").and_then(|v| v.as_str()) {
            Some(p) => p,
            None => raise_error!(
                "ERR_RECONCILER_NO_PATH",
                error = "Le module ne possède pas de chemin physique 'path'."
            ),
        };
        let path = Path::new(path_str);

        // --- 2. LECTURE DU FICHIER ---
        let content = match fs::read_to_string_async(path).await {
            Ok(c) => c,
            Err(e) => raise_error!(
                "ERR_SYSTEM_IO",
                error = e,
                context = json_value!({ "action": "read_module_for_tagging", "path": path_str })
            ),
        };

        let mut edits: Vec<(usize, usize, String)> = Vec::new();

        // =========================================================================
        // 📜 GESTION DU CARTOUCHE (Header Sémantique MBSE)
        // =========================================================================
        let sync_date = LocalClock::now().format("%Y-%m-%d %H:%M").to_string();

        let expected_cartouche = format!(
            "// @raise-cartouche-start\n\
         // ==============================================================================\n\
         // 🧬 MODULE SÉMANTIQUE : {} [id: {}]\n\
         // 📁 CHEMIN PHYSIQUE   : {}\n\
         // 📅 SYNCHRONISATION   : {}\n\
         // 🤖 IA NOTE : Composant du Jumeau Numérique RAISE (Architecture Zéro Dette).\n\
         // ⚠️ AUTO-GÉNÉRÉ : Les ancres sémantiques (@raise-handle) sont gérées par le CLI.\n\
         // ==============================================================================\n\
         // @raise-cartouche-end",
            handle, module_id, path_str, sync_date
        );

        if let Some(start_idx) = content.find("// @raise-cartouche-start") {
            if let Some(end_offset_relative) = content[start_idx..].find("// @raise-cartouche-end")
            {
                let end_idx = start_idx + end_offset_relative + "// @raise-cartouche-end".len();
                let current_cartouche = &content[start_idx..end_idx];

                if current_cartouche != expected_cartouche {
                    let mut len_to_replace = end_idx - start_idx;
                    if let Some(&b'\n') = content.as_bytes().get(end_idx) {
                        len_to_replace += 1;
                    }
                    edits.push((
                        start_idx,
                        len_to_replace,
                        format!("{}\n", expected_cartouche),
                    ));
                }
            }
        } else {
            edits.push((0, 0, format!("{}\n\n", expected_cartouche)));
        }

        let mut lexer = Lexer::new(&content);
        let tokens = lexer.tokenize();

        let mut all_existing_tags: UniqueSet<usize> = UniqueSet::new();
        let mut used_tags: UniqueSet<usize> = UniqueSet::new();

        for token in &tokens {
            if let Token::LineComment(c) = token {
                if c.starts_with("// @raise-handle:") {
                    let offset = (c.as_ptr() as usize) - (content.as_ptr() as usize);
                    all_existing_tags.insert(offset);
                }
            }
        }

        // =========================================================================
        // 🎯 L'OPTION B : TRAQUEUR DE PORTÉE RÉCURSIF (Scope Stack Zéro Dette)
        // =========================================================================
        #[derive(Debug, Clone, PartialEq)]
        enum BlockType {
            Mod(String),
            Impl,
            Trait,
            Fn,
            Struct,
            Enum,
            Macro,
            Other,
        }

        let mut block_stack: Vec<BlockType> = Vec::new();
        let mut pending_block: Option<BlockType> = None;
        let mut in_signature = false;
        let mut element_start_idx = 0;
        let mut i = 0;

        while i < tokens.len() {
            let token = &tokens[i];

            match token {
                Token::Symbol('{') => {
                    block_stack.push(pending_block.unwrap_or(BlockType::Other));
                    pending_block = None;
                    in_signature = false;
                    element_start_idx = i + 1;
                }
                Token::Symbol('}') => {
                    block_stack.pop();
                    pending_block = None;
                    in_signature = false;
                    element_start_idx = i + 1;
                }
                Token::Symbol(';') => {
                    pending_block = None;
                    in_signature = false;
                    element_start_idx = i + 1;
                }
                Token::Ident(kw) => {
                    // Préparation de la détection de déclaration structurelle
                    let kw_block = match *kw {
                        "mod" => Some(BlockType::Mod(String::new())),
                        "impl" => Some(BlockType::Impl),
                        "trait" => Some(BlockType::Trait),
                        "fn" => Some(BlockType::Fn),
                        "struct" => Some(BlockType::Struct),
                        "enum" | "union" => Some(BlockType::Enum),
                        "macro_rules" => Some(BlockType::Macro),
                        _ => None,
                    };

                    if let Some(b) = kw_block {
                        if pending_block.is_none() {
                            pending_block = Some(b);
                        }
                    }

                    // 🛡️ CONDITIONS DE RECURSIVITÉ (Option B) :
                    // Un élément est tagable s'il se trouve à la racine, dans un mod, un impl ou un trait.
                    // Si on est dans un `Fn` (fonction) ou `Other` (struct), on ignore ses enfants.
                    let is_valid_parent = block_stack.is_empty()
                        || matches!(
                            block_stack.last().unwrap(),
                            BlockType::Mod(_) | BlockType::Impl | BlockType::Trait
                        );

                    let is_target = !in_signature
                        && match *kw {
                            "struct" | "enum" | "impl" | "trait" | "type" | "macro_rules"
                            | "mod" | "fn" => is_valid_parent,
                            _ => false,
                        };

                    if is_target {
                        in_signature = true; // Empêche le taggage accidentel de l'intérieur de la signature (ex: impl Trait)
                        let mut name = String::new();
                        let mut k = i + 1;

                        if *kw == "impl" {
                            let mut trait_name = String::new();
                            let mut target_name = String::new();
                            let mut has_for = false;

                            while k < tokens.len() {
                                match &tokens[k] {
                                    Token::Symbol('{') | Token::Symbol(';') => break,
                                    Token::Ident(n) => {
                                        if *n == "for" {
                                            has_for = true;
                                        } else if has_for && target_name.is_empty() {
                                            target_name = n.to_string();
                                        } else if !has_for && trait_name.is_empty() {
                                            trait_name = n.to_string();
                                        }
                                    }
                                    _ => {}
                                }
                                k += 1;
                            }

                            if has_for && !target_name.is_empty() {
                                name = format!("{}_{}", target_name, trait_name);
                            } else {
                                name = trait_name;
                            }
                        } else {
                            while k < tokens.len() {
                                if let Token::Ident(n) = &tokens[k] {
                                    name = n.to_string();
                                    break;
                                }
                                k += 1;
                            }
                        }

                        if !name.is_empty() {
                            // Si c'est un module, on stocke son nom pour la pile
                            if *kw == "mod" {
                                pending_block = Some(BlockType::Mod(name.clone()));
                            }

                            // 🎯 Traque dynamique du contexte de Test
                            let in_test_scope = block_stack
                                .iter()
                                .any(|b| matches!(b, BlockType::Mod(m) if m == "tests"));

                            let mut tag_type = match *kw {
                                "fn" => "fn",
                                "struct" => "struct",
                                "enum" | "union" => "enum",
                                "impl" => "impl",
                                "trait" => "trait",
                                "type" => "type",
                                "macro_rules" => "macro",
                                "mod" => "mod",
                                _ => "unknown",
                            };

                            if in_test_scope && *kw == "fn" {
                                tag_type = "test";
                            }

                            let expected_tag_content =
                                format!("// @raise-handle: {}:{}", tag_type, name);

                            let mut found_existing_tag = None;
                            for token in &tokens[element_start_idx..i] {
                                if let Token::LineComment(c) = token {
                                    if c.starts_with("// @raise-handle:") {
                                        found_existing_tag = Some(*c);
                                        break;
                                    }
                                }
                            }

                            if let Some(existing_c) = found_existing_tag {
                                let offset =
                                    (existing_c.as_ptr() as usize) - (content.as_ptr() as usize);
                                used_tags.insert(offset);

                                if existing_c != expected_tag_content {
                                    edits.push((offset, existing_c.len(), expected_tag_content));
                                }
                            } else {
                                let mut insert_token_idx = i;
                                for (offset, token) in
                                    tokens[element_start_idx..i].iter().enumerate()
                                {
                                    match token {
                                        Token::Whitespace(_) => continue,
                                        Token::LineComment(c) if !c.starts_with("///") => continue,
                                        Token::BlockComment(_) => continue,
                                        _ => {
                                            insert_token_idx = element_start_idx + offset;
                                            break;
                                        }
                                    }
                                }

                                let mut offset = 0;
                                for k in insert_token_idx..tokens.len() {
                                    let s = tokens[k].as_str();
                                    if !s.is_empty() {
                                        offset =
                                            (s.as_ptr() as usize) - (content.as_ptr() as usize);
                                        for token in &tokens[insert_token_idx..k] {
                                            if let Token::Symbol(c) = token {
                                                offset -= c.len_utf8();
                                            }
                                        }
                                        break;
                                    }
                                }

                                let expected_tag_with_newline =
                                    format!("{}\n", expected_tag_content);
                                edits.push((offset, 0, expected_tag_with_newline));
                            }
                        }
                    }
                }
                _ => {}
            }
            i += 1;
        }

        // --- 3. GARBAGE COLLECTOR : Nettoyage des fantômes et anciennes scories ---
        for offset in all_existing_tags.iter() {
            if !used_tags.contains(offset) {
                for token in &tokens {
                    if let Token::LineComment(c) = token {
                        let t_offset = (c.as_ptr() as usize) - (content.as_ptr() as usize);
                        if t_offset == *offset {
                            let mut len_to_remove = c.len();
                            if let Some(&b'\n') = content.as_bytes().get(t_offset + len_to_remove) {
                                len_to_remove += 1;
                            }
                            edits.push((*offset, len_to_remove, String::new()));
                            break;
                        }
                    }
                }
            }
        }

        // 🧹 Éradication de l'ancien en-tête manuel
        for token in &tokens {
            if let Token::LineComment(c) = token {
                if c.starts_with("// FICHIER") {
                    let offset = (c.as_ptr() as usize) - (content.as_ptr() as usize);
                    let mut len_to_remove = c.len();
                    if let Some(&b'\n') = content.as_bytes().get(offset + len_to_remove) {
                        len_to_remove += 1;
                    }
                    edits.push((offset, len_to_remove, String::new()));
                }
            }
        }

        // --- 4. APPLICATION DES ÉDITS IN-PLACE ---
        let edits_count = edits.len();
        if edits_count > 0 {
            edits.sort_by(|a, b| {
                let cmp = b.0.cmp(&a.0);
                if cmp == FmtOrdering::Equal {
                    b.1.cmp(&a.1)
                } else {
                    cmp
                }
            });
            let mut modified_content = content.clone();

            for (offset, len_to_remove, new_text) in edits {
                if len_to_remove > 0 {
                    modified_content.replace_range(offset..(offset + len_to_remove), &new_text);
                } else {
                    modified_content.insert_str(offset, &new_text);
                }
            }

            if let Err(e) = fs::write_async(path, &modified_content).await {
                raise_error!("ERR_AUTO_TAG_WRITE_FAILED", error = e);
            }
            let _ = crate::utils::io::os::exec_command_async(
                "rustfmt",
                &["--edition", "2021", path.to_string_lossy().as_ref()],
                None,
            )
            .await;
        }

        Ok(edits_count)
    }
}

// =========================================================================
// TESTS UNITAIRES (Fiabilisés)
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_reconciler_ast_perfect_extraction() {
        let code = r#"
// @raise-handle: fn:complex_logic
/// Doc ligne 1
/// Doc ligne 2
#[async_test]
#[cfg(feature = "ai")]
pub async fn complex_logic() -> Result<(), Error> {
    let a = 1;
}
"#;
        let elements = Reconciler::parse_content(code, "test_module_id".to_string()).unwrap();
        assert_eq!(elements.len(), 1);
        let el = &elements[0];

        assert_eq!(el.handle, "fn:complex_logic");
        assert_eq!(el.visibility, Visibility::Public);
        assert_eq!(el.element_type, CodeElementType::Function);
        assert_eq!(el.docs.as_deref().unwrap(), "Doc ligne 1\nDoc ligne 2");
        assert_eq!(
            el.attributes,
            vec!["#[async_test]", "#[cfg(feature = \"ai\")]"]
        );
        assert_eq!(
            el.signature,
            "pub async fn complex_logic() -> Result<(), Error>"
        );
        assert_eq!(el.body.as_deref().unwrap(), "{\n    let a = 1;\n}");
    }

    #[test]
    fn test_reconciler_lexer_destroys_string_brace_bug() {
        let code = r#"
// @raise-handle: fn:trap
fn trap() {
    let s = "{ une accolade piège }"; // Un commentaire avec {
    /* Un bloc avec } */
    let c = '{';
}
"#;
        let elements = Reconciler::parse_content(code, "test_module_id".to_string()).unwrap();
        assert_eq!(elements.len(), 1);

        let el = &elements[0];
        assert!(el
            .body
            .as_deref()
            .unwrap()
            .contains("{ une accolade piège }"));
        assert!(el.body.as_deref().unwrap().contains("let c = '{';"));
    }

    #[test]
    fn test_reconciler_zero_copy_raw_strings() {
        let code = r##"
// @raise-handle: fn:raw_string_test
fn raw_string_test() {
    // Si le parseur ne gère pas les raw strings, cette accolade désynchronise le compteur
    let regex = r#"(?x) { \d+ }"#; 
}
"##;
        let elements = Reconciler::parse_content(code, "test_module_id".to_string()).unwrap();
        assert_eq!(
            elements.len(),
            1,
            "Le parsing ne doit pas échouer sur un Stack Overflow ou une désynchronisation"
        );
        assert!(elements[0]
            .body
            .as_deref()
            .unwrap()
            .contains(r##"r#"(?x) { \d+ }"#"##));
    }

    #[test]
    fn test_reconciler_comments_in_signature() {
        let code = r#"
// @raise-handle: fn:comment_in_sig
pub fn with_comment(
    /* identifiant de session */
    session_id: u32
) {
    println!("ok");
}
"#;
        let elements = Reconciler::parse_content(code, "test_module_id".to_string()).unwrap();
        assert_eq!(elements.len(), 1);

        let el = &elements[0];
        assert!(!el.signature.contains("identifiant de session"));
        assert_eq!(
            el.signature,
            "pub fn with_comment(\n    \n    session_id: u32\n)"
        );
    }

    #[test]
    fn test_reconciler_unbalanced_error_handled_by_raise() {
        let code = r#"
// @raise-handle: fn:broken
fn broken() {
    let a = 1;
// Missing closing brace
"#;
        let result = Reconciler::parse_content(code, "mod_test_broken".to_string());

        assert!(result.is_err(), "Devrait retourner une erreur RAISE");

        if let Err(crate::utils::core::error::AppError::Structured(data)) = result {
            assert_eq!(data.code, "ERR_RECONCILER_UNBALANCED_BRACES");
        } else {
            panic!("Le type d'erreur n'est pas AppError::Structured");
        }
    }

    #[test]
    fn test_reconciler_extracts_test_module_with_attributes() {
        let code = r#"
// @raise-handle: mod:tests
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn internal_test() {
        assert_eq!(1, 1);
    }
}
"#;
        let elements = Reconciler::parse_content(code, "test_module_id".to_string()).unwrap();

        assert_eq!(elements.len(), 1);
        let el = &elements[0];

        assert_eq!(el.handle, "mod:tests");
        assert_eq!(el.element_type, CodeElementType::TestModule);
        assert!(el.attributes.contains(&"#[cfg(test)]".to_string()));

        let body = el.body.as_deref().unwrap();
        assert!(body.contains("fn internal_test()"));
        assert!(body.contains("use super::*;"));
    }

    #[async_test]
    async fn test_auto_tagger_injects_handle_on_test_module() {
        let code = r#"
fn core_logic() {}

#[cfg(test)]
mod tests {
    #[test]
    fn test_core() {}
}
"#;
        let temp_dir_guard =
            crate::utils::io::fs::tempdir().expect("Impossible de créer le tempdir");
        let sandbox_dir = temp_dir_guard.path().join("raise_test_tagger");
        crate::utils::io::fs::ensure_dir_async(&sandbox_dir)
            .await
            .unwrap();

        let path = sandbox_dir.join("test_file.rs");
        crate::utils::io::fs::write_async(&path, code)
            .await
            .unwrap();

        let mock_module_doc = crate::utils::data::json::json_value!({
            "_id": "mod_test_id",
            "handle": "mod_test",
            "path": path.to_string_lossy().to_string()
        });

        let edits_count = Reconciler::auto_tag_module(&mock_module_doc).await.unwrap();

        assert!(edits_count > 0);

        let modified_code = crate::utils::io::fs::read_to_string_async(&path)
            .await
            .unwrap();

        assert!(modified_code.contains("// @raise-handle: mod:tests\n#[cfg(test)]\nmod tests {"));
    }

    #[test]
    fn test_reconciler_extracts_internal_dependencies() {
        let code = r#"
// @raise-handle: mod:tests
#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::prelude::*;
    use std::collections::{HashMap, HashSet};

    #[test]
    fn some_test() {}
}
"#;
        let elements = Reconciler::parse_content(code, "test_module_id".to_string()).unwrap();

        assert_eq!(
            elements.len(),
            1,
            "Le module parent doit être extrait comme un seul élément unifié"
        );
        let el = &elements[0];

        assert_eq!(el.handle, "mod:tests");

        let imports = el
            .metadata
            .get("internal_imports")
            .expect("Les métadonnées 'internal_imports' sont absentes");

        assert!(
            imports.contains("super::*"),
            "La dépendance relative 'super::*' n'a pas été extraite"
        );
        assert!(
            imports.contains("crate::utils::prelude::*"),
            "La dépendance absolue n'a pas été extraite"
        );
        assert!(
            imports.contains("std::collections::{HashMap, HashSet}"),
            "La dépendance destructurée n'a pas été correctement formatée"
        );

        assert!(
            el.dependencies.is_empty(),
            "Le vecteur de dépendances sémantiques doit rester vierge"
        );
    }
}
