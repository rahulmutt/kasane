use std::io::Write;

fn add<W: Write + std::io::Seek>(w: &mut zip::ZipWriter<W>, name: &str, data: &[u8]) {
    let opts =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    w.start_file(name, opts).unwrap();
    w.write_all(data).unwrap();
}

fn main() {
    let mut buf = std::io::Cursor::new(Vec::new());
    let mut w = zip::ZipWriter::new(&mut buf);
    add(&mut w, "[Content_Types].xml", b"<Types/>");
    add(&mut w, "ppt/presentation.xml", br#"<p:presentation xmlns:r="r"><p:sldIdLst><p:sldId r:id="rId2"/><p:sldId r:id="rId3"/></p:sldIdLst></p:presentation>"#);
    add(&mut w, "ppt/_rels/presentation.xml.rels", br#"<Relationships><Relationship Id="rId2" Type="x/slide" Target="slides/slide1.xml"/><Relationship Id="rId3" Type="x/slide" Target="slides/slide2.xml"/></Relationships>"#);
    add(&mut w, "ppt/slides/slide1.xml", br#"<p:sld xmlns:a="a" xmlns:p="p" xmlns:r="r"><p:cSld><p:spTree><p:sp><p:nvSpPr><p:nvPr><p:ph type="title"/></p:nvPr></p:nvSpPr><p:txBody><a:p><a:r><a:t>Welcome</a:t></a:r></a:p></p:txBody></p:sp><p:sp><p:nvSpPr><p:nvPr><p:ph type="body"/></p:nvPr></p:nvSpPr><p:txBody><a:p><a:r><a:t>First point</a:t></a:r></a:p><a:p><a:pPr lvl="1"/><a:r><a:t>Sub point</a:t></a:r></a:p></p:txBody></p:sp><p:pic><p:nvPicPr><p:cNvPr id="5" name="P" descr="a diagram"/></p:nvPicPr><p:blipFill><a:blip r:embed="rId9"/></p:blipFill></p:pic></p:spTree></p:cSld></p:sld>"#);
    add(&mut w, "ppt/slides/_rels/slide1.xml.rels", br#"<Relationships><Relationship Id="rId9" Type="x/image" Target="../media/image1.png"/><Relationship Id="rId8" Type="x/notesSlide" Target="../notesSlides/notesSlide1.xml"/></Relationships>"#);
    add(&mut w, "ppt/slides/slide2.xml", br#"<p:sld xmlns:a="a" xmlns:p="p"><p:cSld><p:spTree><p:sp><p:nvSpPr><p:nvPr><p:ph type="title"/></p:nvPr></p:nvSpPr><p:txBody><a:p><a:r><a:t>Data</a:t></a:r></a:p></p:txBody></p:sp><p:graphicFrame><a:graphic><a:graphicData><a:tbl><a:tr><a:tc><a:txBody><a:p><a:r><a:t>Name</a:t></a:r></a:p></a:txBody></a:tc><a:tc><a:txBody><a:p><a:r><a:t>Value</a:t></a:r></a:p></a:txBody></a:tc></a:tr><a:tr><a:tc><a:txBody><a:p><a:r><a:t>x</a:t></a:r></a:p></a:txBody></a:tc><a:tc><a:txBody><a:p><a:r><a:t>1</a:t></a:r></a:p></a:txBody></a:tc></a:tr></a:tbl></a:graphicData></a:graphic></p:graphicFrame></p:spTree></p:cSld></p:sld>"#);
    add(&mut w, "ppt/notesSlides/notesSlide1.xml", br#"<p:notes xmlns:a="a" xmlns:p="p"><p:cSld><p:spTree><p:sp><p:nvSpPr><p:nvPr><p:ph type="body"/></p:nvPr></p:nvSpPr><p:txBody><a:p><a:r><a:t>Remember to smile.</a:t></a:r></a:p></p:txBody></p:sp></p:spTree></p:cSld></p:notes>"#);
    add(
        &mut w,
        "ppt/media/image1.png",
        b"\x89PNG\r\n\x1a\nFAKEPNGDATA",
    );
    w.finish().unwrap();
    std::fs::create_dir_all("tests/fixtures/pptx").unwrap();
    std::fs::write("tests/fixtures/pptx/minimal.pptx", buf.into_inner()).unwrap();
    println!("wrote tests/fixtures/pptx/minimal.pptx");
}
