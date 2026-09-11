# Strict, bounded v2 bootstrap subset: no escaped keys/strings, duplicates, arrays or unknown fields.
function fail(){bad=1; print "invalid desktop release metadata" > "/dev/stderr"; exit 1}
function ws(){sub(/^[ \t\r\n]*/,"",input)}
function take(c){ws();if(substr(input,1,1)!=c)fail();input=substr(input,2)}
function str( v){ws();if(!match(input,/^"[A-Za-z0-9_.+\/-]+"/))fail();v=substr(input,2,RLENGTH-2);input=substr(input,RLENGTH+1);return v}
function object(prefix,depth, key,path,c){
 if(depth>3)fail();take("{");ws();if(substr(input,1,1)=="}")fail();
 while(1){key=str();path=prefix key;if(seen[path]++)fail();take(":");ws();c=substr(input,1,1);
 if(c=="{")object(path ".",depth+1);else if(c=="\"")value[path]=str();else if(substr(input,1,4)=="null"){value[path]="null";input=substr(input,5)}else if(c=="2"){value[path]="2";input=substr(input,2)}else fail();
 ws();c=substr(input,1,1);input=substr(input,2);if(c=="}")break;if(c!=",")fail(); }
}
{input=input $0 "\n";if(length(input)>65536)fail()}
END{if(bad)exit 1;object("",0);ws();if(length(input))fail();
 keys="format_version version target channel platform platform.archive platform.sha256 model model.name model.revision model.archive model.sha256 model.members model.members.models/minilm/model.onnx model.members.models/minilm/tokenizer.json model.members.models/minilm/LICENSE model.members.models/minilm/README.md signature";
 split(keys,a," ");for(i in a)want[a[i]]=1;for(k in seen)if(!want[k])fail();for(k in want)if(!seen[k])fail();
 if(value["format_version"]!="2"||value["target"]!=target||value["signature"]!="null"||(value["channel"]!="stable"&&value["channel"]!="dev"))fail();
 if(value["version"]!~/^[0-9]+\.[0-9]+\.[0-9]+([-+][A-Za-z0-9.-]+)?$/)fail();
 if(value["platform.archive"]!="hieronymus-"value["version"]"-"target".tar.gz")fail();
 if(value["model.name"]!="paraphrase-multilingual-MiniLM-L12-v2"||value["model.revision"]!="e8f8c211226b894fcb81acc59f3b34ba3efd5f42")fail();
 if(value["model.archive"]!="hieronymus-model-"value["model.name"]"-"value["model.revision"]".tar.gz")fail();
 for(k in value)if(k~/sha256$/||k~/^model.members./)if(length(value[k])!=64||value[k]~/[^a-f0-9]/)fail();
 if(value["model.members.models/minilm/model.onnx"]!="10f7a088420252b26caf819236ca2c9d2987afd0fc06fec7553b542a5655a05a"||value["model.members.models/minilm/tokenizer.json"]!="2c3387be76557bd40970cec13153b3bbf80407865484b209e655e5e4729076b8"||value["model.members.models/minilm/LICENSE"]!="cfc7749b96f63bd31c3c42b5c471bf756814053e847c10f3eb003417bc523d30"||value["model.members.models/minilm/README.md"]!="1e98ea05b0de579fcaad3d625b62ea55647142ed674d5f5ebf1440e4bbbb6f23")fail();
 print value["version"];print value["platform.archive"];print value["platform.sha256"];print value["model.archive"];print value["model.sha256"];
}
