use std::borrow::Cow;
use std::fmt::Debug;
use std::fmt::Formatter;
use std::fmt::Result as FmtResult;
use std::path::Path;
use std::path::PathBuf;
use std::rc::Rc;

use crate::inspect::FindAddrOpts;
use crate::inspect::SymInfo;
use crate::symbolize::AddrCodeInfo;
use crate::symbolize::IntSym;
use crate::symbolize::SrcLang;
use crate::Addr;
use crate::Result;
use crate::SymResolver;

use super::types::STT_FUNC;
use super::ElfBackend;
use super::ElfParser;

/// The symbol resolver for a single ELF file.
///
/// An ELF file may be loaded into an address space with a relocation.
/// The callers should provide the path of an ELF file and where it's
/// executable segment(s) is loaded.
///
/// For some ELF files, they are located at a specific address
/// determined during compile-time.  For these cases, just pass `0` as
/// it's loaded address.
pub struct ElfResolver {
    backend: ElfBackend,
    file_name: PathBuf,
}

impl ElfResolver {
    pub(crate) fn with_backend(file_name: &Path, backend: ElfBackend) -> Result<ElfResolver> {
        Ok(ElfResolver {
            backend,
            file_name: file_name.to_path_buf(),
        })
    }

    pub(crate) fn parser(&self) -> &Rc<ElfParser> {
        match &self.backend {
            #[cfg(feature = "dwarf")]
            ElfBackend::Dwarf(dwarf) => dwarf.parser(),
            ElfBackend::Elf(parser) => parser,
        }
    }

    /// Retrieve the path to the ELF file represented by this resolver.
    pub(crate) fn file_name(&self) -> &Path {
        &self.file_name
    }
}

impl SymResolver for ElfResolver {
    #[cfg_attr(feature = "tracing", crate::log::instrument(fields(addr = format_args!("{addr:#x}"))))]
    fn find_sym(&self, addr: Addr) -> Result<Option<IntSym<'_>>> {
        #[cfg(feature = "dwarf")]
        if let ElfBackend::Dwarf(dwarf) = &self.backend {
            if let Some(sym) = dwarf.find_sym(addr)? {
                return Ok(Some(sym))
            }
        }

        let parser = self.parser();
        if let Some((name, addr, size)) = parser.find_sym(addr, STT_FUNC)? {
            // ELF does not carry any source code language information.
            let lang = SrcLang::Unknown;
            // We found the address in ELF.
            // TODO: Long term we probably want a different heuristic here, as
            //       there can be valid differences between the two formats
            //       (e.g., DWARF could contain more symbols).
            let sym = IntSym {
                name,
                addr,
                size: Some(size),
                lang,
            };
            Ok(Some(sym))
        } else {
            Ok(None)
        }
    }

    fn find_addr<'slf>(&'slf self, name: &str, opts: &FindAddrOpts) -> Result<Vec<SymInfo<'slf>>> {
        fn find_addr_impl<'slf>(
            slf: &'slf ElfResolver,
            name: &str,
            opts: &FindAddrOpts,
        ) -> Result<Vec<SymInfo<'slf>>> {
            #[cfg(feature = "dwarf")]
            if let ElfBackend::Dwarf(dwarf) = &slf.backend {
                let syms = dwarf.find_addr(name, opts)?;
                if !syms.is_empty() {
                    return Ok(syms)
                }
            }

            let parser = slf.parser();
            let syms = parser.find_addr(name, opts)?;
            Ok(syms)
        }

        let mut syms = find_addr_impl(self, name, opts)?;
        let () = syms
            .iter_mut()
            .for_each(|sym| sym.obj_file_name = Some(Cow::Borrowed(&self.file_name)));
        Ok(syms)
    }

    #[cfg(feature = "dwarf")]
    fn find_code_info(&self, addr: Addr, inlined_fns: bool) -> Result<Option<AddrCodeInfo<'_>>> {
        if let ElfBackend::Dwarf(dwarf) = &self.backend {
            dwarf.find_code_info(addr, inlined_fns)
        } else {
            Ok(None)
        }
    }

    #[cfg(not(feature = "dwarf"))]
    fn find_code_info(&self, addr: Addr, inlined_fns: bool) -> Result<Option<AddrCodeInfo<'_>>> {
        Ok(None)
    }
}

impl Debug for ElfResolver {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        match self.backend {
            #[cfg(feature = "dwarf")]
            ElfBackend::Dwarf(_) => write!(f, "DWARF {}", self.file_name.display()),
            ElfBackend::Elf(_) => write!(f, "ELF {}", self.file_name.display()),
        }
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    use std::path::Path;

    #[cfg(feature = "dwarf")]
    use crate::dwarf::DwarfResolver;


    /// Exercise the `Debug` representation of various types.
    #[test]
    fn debug_repr() {
        let path = Path::new(&env!("CARGO_MANIFEST_DIR"))
            .join("data")
            .join("test-stable-addresses.bin");

        let parser = Rc::new(ElfParser::open(&path).unwrap());
        let backend = ElfBackend::Elf(parser.clone());
        let resolver = ElfResolver::with_backend(&path, backend).unwrap();
        let dbg = format!("{resolver:?}");
        assert!(dbg.starts_with("ELF"), "{dbg}");
        assert!(dbg.ends_with("test-stable-addresses.bin"), "{dbg}");

        #[cfg(feature = "dwarf")]
        {
            let dwarf = DwarfResolver::from_parser(parser, true).unwrap();
            let backend = ElfBackend::Dwarf(Rc::new(dwarf));
            let resolver = ElfResolver::with_backend(&path, backend).unwrap();
            let dbg = format!("{resolver:?}");
            assert!(dbg.starts_with("DWARF"), "{dbg}");
            assert!(dbg.ends_with("test-stable-addresses.bin"), "{dbg}");
        }
    }

    /// Check that we fail finding an offset for an address not
    /// representing a symbol in an ELF file.
    #[test]
    fn addr_without_offset() {
        let path = Path::new(&env!("CARGO_MANIFEST_DIR"))
            .join("data")
            .join("test-stable-addresses-no-dwarf.bin");
        let parser = ElfParser::open(&path).unwrap();

        assert_eq!(parser.find_file_offset(0x0).unwrap(), None);
        assert_eq!(parser.find_file_offset(0xffffffffffffffff).unwrap(), None);
    }
}
