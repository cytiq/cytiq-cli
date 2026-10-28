#!/usr/bin/env Rscript
# Independent black-box numerical fixtures. Do not change these to accommodate
# Rust failures. Regenerate only after changing this generator or the R pin.
suppressPackageStartupMessages(library(jsonlite))
cases <- list(
  list(x=c(0,1,2,3), y=c(0,2,3,4), query=seq(-1,4,length.out=101)),
  list(x=c(-3,-1,-1,0,1,4), y=c(-2,-1,0,0,0.01,10), query=seq(-5,6,length.out=101)),
  list(x=c(0,0.001,1,2,4), y=c(0,10,10.01,10.02,100), query=seq(-0.01,4.01,length.out=151))
)
cases <- lapply(cases, function(z) {
  z$expected <- suppressWarnings(splinefun(z$x,z$y,method="monoH.FC")(z$query))
  z
})
quantiles <- lapply(list(c(-10,-1,0,0,0,1,100), seq(-1,1,length.out=1000)), function(x) {
  p <- c(0,0.001,seq(0.01,0.99,0.01),0.999,1)
  list(sorted=x, probabilities=p, expected=unname(quantile(x,p,type=7)))
})
# Same codebook and explicit resampling schedules are supplied to Rust and R.
codes <- cbind(rep(c(0,10,25),each=3)+rep(c(0,0.1,0.3),3),rep(c(0,2,-2),each=3))
set.seed(7)
schedules <- replicate(100,sort(sample.int(nrow(codes),8)),simplify=FALSE)
seen <- together <- matrix(0,nrow(codes),nrow(codes))
for (ids in schedules) {
  labels <- cutree(hclust(dist(codes[ids,]),method="average"),k=3)
  seen[ids,ids] <- seen[ids,ids]+1
  together[ids,ids] <- together[ids,ids]+outer(labels,labels,"==")
}
consensus <- ifelse(seen>0,together/pmax(seen,1),0)
diag(consensus)<-1
labels <- unname(cutree(hclust(as.dist(1-consensus),method="average"),k=3))
fixture <- list(r_version=R.version.string, splines=cases, quantiles=quantiles,
                consensus=list(codebook=codes,k=3,schedules=lapply(schedules,function(x)x-1L),labels=labels))
dir.create("data/fixtures/cytonorm",recursive=TRUE,showWarnings=FALSE)
write_json(fixture,"data/fixtures/cytonorm/numeric.json",digits=17,pretty=TRUE,auto_unbox=TRUE)
