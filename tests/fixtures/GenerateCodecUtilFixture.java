// Fixture generator for Rucene CodecUtil portability tests.
//
// This program uses Apache Lucene Core 10.5.0 CodecUtil to produce a known
// binary file containing a codec header/footer and its CRC-32 checksum. The
// values it prints are embedded as reference data in
// tests/codecs_portability.rs. Re-run this generator whenever the header/footer
// shape changes and copy the new constants into the Rust tests.
//
// Build requirements:
//   - JDK 21 (or compatible)
//   - lucene-core-10.5.0.jar
//
// Example usage:
//   javac -cp lucene-core-10.5.0.jar GenerateCodecUtilFixture.java
//   java -cp ".:lucene-core-10.5.0.jar" GenerateCodecUtilFixture

import org.apache.lucene.codecs.CodecUtil;
import org.apache.lucene.store.ByteBuffersDirectory;
import org.apache.lucene.store.Directory;
import org.apache.lucene.store.IndexOutput;
import org.apache.lucene.store.IndexInput;
import org.apache.lucene.store.IOContext;
import java.util.Base64;

public class GenerateCodecUtilFixture {
    public static void main(String[] args) throws Exception {
        try (Directory dir = new ByteBuffersDirectory()) {
            try (IndexOutput out = dir.createOutput("codecutil.bin", IOContext.DEFAULT)) {
                CodecUtil.writeHeader(out, "RuceneCodecUtil", 42);
                out.writeVInt(12345);
                out.writeString("payload");
                CodecUtil.writeFooter(out);
            }

            byte[] fileBytes;
            try (IndexInput in = dir.openInput("codecutil.bin", IOContext.DEFAULT)) {
                fileBytes = new byte[(int) in.length()];
                in.readBytes(fileBytes, 0, fileBytes.length);
            }

            System.out.println("header_footer_base64=" + Base64.getEncoder().encodeToString(fileBytes));
            System.out.println("header_footer_length=" + fileBytes.length);

            try (IndexOutput out = dir.createOutput("idxheader.bin", IOContext.DEFAULT)) {
                byte[] id = new byte[16];
                for (int i = 0; i < 16; i++) {
                    id[i] = (byte) i;
                }
                CodecUtil.writeIndexHeader(out, "RuceneIdx", 7, id, "_0");
                out.writeByte((byte) 0x99);
                CodecUtil.writeFooter(out);
            }

            try (IndexInput in = dir.openInput("idxheader.bin", IOContext.DEFAULT)) {
                fileBytes = new byte[(int) in.length()];
                in.readBytes(fileBytes, 0, fileBytes.length);
            }

            System.out.println("index_header_base64=" + Base64.getEncoder().encodeToString(fileBytes));
            System.out.println("index_header_length=" + fileBytes.length);
        }
    }
}
